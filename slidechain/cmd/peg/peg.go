package main

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/hex"
	"flag"
	"fmt"
	"log"
	"math"
	"net/http"
	"strconv"
	"strings"
	"time"

	"github.com/chain/txvm/crypto/ed25519"
	"github.com/chain/txvm/protocol/bc"
	"github.com/chain/txvm/protocol/txvm"
	"github.com/chain/txvm/protocol/txvm/asm"
	"github.com/interstellar/slingshot/slidechain"
	"github.com/interstellar/slingshot/slidechain/stellar"
	"github.com/pkg/errors"
	"github.com/stellar/go/clients/horizon"
	"github.com/stellar/go/xdr"
)

func main() {
	var (
		custodian  = flag.String("custodian", "", "Stellar account ID of custodian account")
		amount     = flag.String("amount", "", "amount to peg, in lumens")
		recipient  = flag.String("recipient", "", "hex-encoded txvm public key for the recipient of the pegged funds")
		seed       = flag.String("seed", "", "seed of Stellar source account")
		horizonURL = flag.String("horizon", "https://horizon-testnet.stellar.org", "horizon URL")
		code       = flag.String("code", "", "asset code for non-Lumen asset")
		issuer     = flag.String("issuer", "", "asset issuer for non-Lumen asset")
		dbfile     = flag.String("db", "slidechain.db", "path to db")
	)
	flag.Parse()

	if *amount == "" {
		log.Fatal("must specify peg-in amount")
	}
	if *custodian == "" {
		log.Fatal("must specify custodian account")
	}
	if (*code == "" && *issuer != "") || (*code != "" && *issuer == "") {
		log.Fatal("must specify both code and issuer for non-Lumen asset")
	}
	if *recipient == "" {
		log.Print("no recipient specified, generating txvm keypair...")
		pubkey, privkey, err := ed25519.GenerateKey(nil)
		if err != nil {
			log.Fatalf("error generating txvm keypair: %s", err)
		}
		*recipient = hex.EncodeToString(pubkey)
		log.Printf("pegging funds to keypair %x / %x", privkey, pubkey)
	}
	if *seed == "" {
		log.Print("no seed specified, generating and funding a new account...")
		kp := stellar.NewFundedAccount()
		*seed = kp.Seed()
	}

	if _, err := strconv.ParseFloat(*amount, 64); err != nil {
		log.Printf("invalid amount string %s: %s", *amount, err)
	}

	var recipientPubkey [32]byte
	if len(*recipient) != 64 {
		log.Fatalf("invalid recipient length: got %d want 64", len(*recipient))
	}
	_, err := hex.Decode(recipientPubkey[:], []byte(*recipient))
	if err != nil {
		log.Fatal(err, "decoding recipient")
	}
	// TODO(debnil): Write a new GetCustodian RPC.
	db, err := sql.Open("sqlite3", *dbfile)
	if err != nil {
		log.Fatalf("error opening db: %s", err)
	}
	defer db.Close()

	hclient := &horizon.Client{
		URL:  strings.TrimRight(*horizonURL, "/"),
		HTTP: new(http.Client),
	}
	ctx := context.Background()
	c, err := slidechain.GetCustodian(ctx, db, *horizonURL)
	if err != nil {
		log.Fatalf("error getting custodian: %s", err)
	}
	tx, err := stellar.BuildPegInTx(*seed, recipientPubkey, *amount, *code, *issuer, *custodian, hclient)
	if err != nil {
		log.Fatal(err, "building transaction")
	}
	txenv, err := tx.Sign(*seed)
	if err != nil {
		log.Fatal(err, "signing transaction")
	}
	txstr, err := xdr.MarshalBase64(txenv.E)
	if err != nil {
		log.Fatal(err, "marshaling tx to base64")
	}
	succ, err := hclient.SubmitTransaction(txstr)
	if err != nil {
		log.Fatalf("%s: submitting tx %s", err, txstr)
	}
	log.Printf("successfully submitted peg-in tx hash %s on ledger %d", succ.Hash, succ.Ledger)
	expMS := int64(bc.Millis(time.Now().Add(10 * time.Minute)))
	err = c.RecordPegs(ctx, txenv.E.Tx, recipientPubkey[:], expMS)
	if err != nil {
		log.Fatal(err, "recording pegs")
	}
	log.Printf("successfully record pegs for tx hash %s", succ.Hash)
}

func buildPrepegTx(bcid []byte, amount, expMS int64, assetXDR []byte, pubkey ed25519.PublicKey) ([]byte, error) {
	nonceHash := slidechain.UniqueNonceHash(bcid, expMS)
	finalizeExpMS := int64(bc.Millis(time.Now().Add(9 * time.Minute)))
	buf := new(bytes.Buffer)
	// Set up pre-peg tx arg stack: asset, amount, zeroval, {pubkey}, quorum
	fmt.Fprintf(buf, "x'%x' put\n", assetXDR)
	fmt.Fprintf(buf, "%d put\n", amount)
	fmt.Fprintf(buf, "x'%x' put\n", nonceHash)
	fmt.Fprintf(buf, "{x'%x'} put\n", pubkey)
	fmt.Fprintf(buf, "1 put\n") // The signer quorum size of 1 is fixed.
	fmt.Fprintf(buf, "x'%x' %d nonce finalize\n", bcid, finalizeExpMS)
	tx, err := asm.Assemble(buf.String())
	if err != nil {
		return nil, errors.Wrap(err, "assembling pre-peg tx")
	}
	_, err = txvm.Validate(tx, 3, math.MaxInt64)
	if err != nil {
		return nil, errors.Wrap(err, "validating pre-peg tx")
	}
	return tx, nil
}
