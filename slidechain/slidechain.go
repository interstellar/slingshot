package main

import (
	"context"
	"database/sql"
	"encoding/hex"
	"flag"
	"fmt"
	"log"
	"net"
	"net/http"
	"strings"
	"sync"
	"time"

	"github.com/bobg/multichan"
	"github.com/chain/txvm/errors"
	"github.com/chain/txvm/protocol"
	"github.com/chain/txvm/protocol/bc"
	"github.com/chain/txvm/protocol/txvm"
	"github.com/chain/txvm/protocol/txvm/asm"
	i10rnet "github.com/interstellar/starlight/net"
	_ "github.com/mattn/go-sqlite3"
	"github.com/stellar/go/clients/horizon"
	"github.com/stellar/go/xdr"
)

var (
	initialBlock *bc.Block
	chain        *protocol.Chain
)

type custodian struct {
	seed      string
	accountID xdr.AccountId
	db        *sql.DB
	w         *multichan.W
	hclient   *horizon.Client
	imports   *sync.Cond
	exports   *sync.Cond
	network   string
}

func start(ctx context.Context, addr, dbfile, horizonURL string) (*custodian, error) {
	db, err := startdb(dbfile)
	if err != nil {
		return nil, errors.Wrap(err, "starting db")
	}

	hclient := &horizon.Client{
		URL:  strings.TrimRight(horizonURL, "/"),
		HTTP: new(http.Client),
	}

	root, err := hclient.Root()
	if err != nil {
		return nil, errors.Wrap(err, "getting horizon client root")
	}

	custAccountID, err := custodianAccount(ctx, db, hclient)
	if err != nil {
		return nil, errors.Wrap(err, "creating/fetching custodian account")
	}

	// TODO(vniu): set custodian account seed
	return &custodian{
		accountID: *custAccountID, // TODO(tessr): should this field be a pointer to an xdr.AccountID?
		db:        db,
		w:         multichan.New((*bc.Block)(nil)),
		hclient:   hclient,
		imports:   sync.NewCond(new(sync.Mutex)),
		exports:   sync.NewCond(new(sync.Mutex)),
		network:   root.NetworkPassphrase,
	}, nil
}

func startdb(dbfile string) (*sql.DB, error) {
	db, err := sql.Open("sqlite3", dbfile)
	if err != nil {
		return nil, errors.Wrap(err, "opening db")
	}
	err = setSchema(db)
	return db, errors.Wrap(err, "creating schema")
}

func main() {
	ctx := context.Background()

	var (
		addr   = flag.String("addr", "localhost:2423", "server listen address")
		dbfile = flag.String("db", "slidechain.db", "path to db")
		url    = flag.String("horizon", "https://horizon-testnet.stellar.org", "horizon server url")
	)

	flag.Parse()

	c, err := start(ctx, *addr, *dbfile, *url)
	if err != nil {
		log.Fatal(err)
	}
	defer c.db.Close()

	// Assemble issuance TxVM program for custodian.
	hexpubkey, err := convertToHex(c.accountID)
	if err != nil {
		log.Fatal(err)
	}
	issueProgSrc = fmt.Sprintf(issueProgFmt, hexpubkey)
	issueProg, err = asm.Assemble(issueProgSrc)
	if err != nil {
		log.Fatal(err)
	}
	issueSeed = txvm.ContractSeed(issueProg)

	var cur horizon.Cursor
	err = c.db.QueryRow("SELECT cursor FROM custodian").Scan(&cur)
	if err != nil && err != sql.ErrNoRows {
		log.Fatal(err)
	}

	heights := make(chan uint64)
	bs, err := newBlockStore(c.db, heights)
	if err != nil {
		log.Fatal(err)
	}

	initialBlock, err = bs.GetBlock(ctx, 1)
	if err != nil {
		log.Fatal(err)
	}

	chain, err = protocol.NewChain(ctx, initialBlock, bs, heights)
	if err != nil {
		log.Fatal("initializing Chain: ", err)
	}
	_, err = chain.Recover(ctx)
	if err != nil {
		log.Fatal(err)
	}

	initialBlockID := initialBlock.Hash()

	listener, err := net.Listen("tcp", *addr)
	if err != nil {
		log.Fatal(err)
	}

	log.Printf("listening on %s, initial block ID %x", listener.Addr(), initialBlockID.Bytes())

	s := &submitter{w: c.w}

	// Start streaming txs, importing, and exporting
	go func() {
		backoff := i10rnet.Backoff{Base: 100 * time.Millisecond}
		for {
			err := c.hclient.StreamTransactions(ctx, c.accountID.Address(), &cur, c.watchPegs)
			if err != nil {
				log.Println("error streaming from horizon: ", err)
			}
			time.Sleep(backoff.Next())
		}
	}()

	go func() {
		err := c.importFromPegs(ctx, s)
		if err != nil {
			log.Fatal("error importing from pegs: ", err)
		}
	}()

	go func() {
		err := c.watchExports(ctx)
		if err != nil {
			log.Fatal("error watching for export txs: ", err)
		}
	}()

	go func() {
		err := c.pegOutFromExports(ctx)
		if err != nil {
			log.Fatal("error pegging out from exports: ", err)
		}
	}()

	http.Handle("/submit", s)
	http.HandleFunc("/get", get)
	http.Serve(listener, nil)
}

func convertToHex(accountID xdr.AccountId) (string, error) {
	bytes, err := accountID.MarshalBinary()
	if err != nil {
		return "", err
	}
	return hex.EncodeToString(bytes), nil
}

func setSchema(db *sql.DB) error {
	_, err := db.Exec(schema)
	return errors.Wrap(err, "creating db schema")
}

func httpErrf(w http.ResponseWriter, code int, msgfmt string, args ...interface{}) {
	http.Error(w, fmt.Sprintf(msgfmt, args...), code)
	log.Printf(msgfmt, args...)
}
