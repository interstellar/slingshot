package main

import (
	"context"
	"database/sql"
	"flag"
	"fmt"
	"log"
	"net"
	"net/http"
	"strings"
	"sync"

	"github.com/bobg/multichan"
	"github.com/chain/txvm/errors"
	"github.com/chain/txvm/protocol"
	"github.com/chain/txvm/protocol/bc"
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

func start(addr, dbfile, custID, horizonURL string) (*custodian, error) {
	db, err := sql.Open("sqlite3", dbfile)
	if err != nil {
		return nil, errors.Wrap(err, "error opening db")
	}

	hclient := &horizon.Client{
		URL:  strings.TrimRight(horizonURL, "/"),
		HTTP: new(http.Client),
	}

	root, err := hclient.Root()
	if err != nil {
		return nil, errors.Wrap(err, "error getting horizon client root")
	}

	var custAccountID xdr.AccountId
	err = custAccountID.SetAddress(custID)
	if err != nil {
		return nil, errors.Wrap(err, "error setting custodian account ID")
	}

	// TODO(vniu): set custodian account seed
	return &custodian{
		accountID: custAccountID,
		db:        db,
		w:         multichan.New((*bc.Block)(nil)),
		hclient:   hclient,
		imports:   sync.NewCond(new(sync.Mutex)),
		exports:   sync.NewCond(new(sync.Mutex)),
		network:   root.NetworkPassphrase,
	}, nil
}

func main() {
	ctx := context.Background()

	var (
		addr   = flag.String("addr", "localhost:2423", "server listen address")
		dbfile = flag.String("db", "slidechain.db", "path to db")
		custID = flag.String("custid", "", "custodian's Stellar account ID")
		url    = flag.String("horizon", "https://horizon-testnet.stellar.org", "horizon server url")
	)

	flag.Parse()

	var cur horizon.Cursor // TODO: initialize from db (if applicable)

	c, err := start(*addr, *dbfile, *custID, *url)
	if err != nil {
		log.Fatal(err)
	}
	defer c.db.Close()

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
		err := c.hclient.StreamTransactions(ctx, *custID, &cur, c.watchPegs)
		if err != nil {
			// TODO: error handling
		}
	}()

	go func() {
		err := c.importFromPegs(ctx, s)
		if err != nil {
			// TODO(vniu): error handling
		}
	}()

	go c.watchExports(ctx)

	go func() {
		err := c.pegOutFromExports(ctx)
		if err != nil {
			// TODO(vniu): error handling
		}
	}()

	http.Handle("/submit", s)
	http.HandleFunc("/get", get)
	http.Serve(listener, nil)
}

func httpErrf(w http.ResponseWriter, code int, msgfmt string, args ...interface{}) {
	http.Error(w, fmt.Sprintf(msgfmt, args...), code)
	log.Printf(msgfmt, args...)
}
