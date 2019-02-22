package slidechain

import (
	"context"
	"database/sql"
	"fmt"
	"io/ioutil"
	"log"
	"os"
	"testing"
	"time"

	"github.com/interstellar/slingshot/slidechain/mockhorizon"
	"github.com/interstellar/slingshot/slidechain/stellar"
	"github.com/stellar/go/clients/horizon"
	"github.com/stellar/go/keypair"
	"github.com/stellar/go/xdr"
)

func TestPegOut(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	testdir, err := ioutil.TempDir("", t.Name())
	if err != nil {
		t.Fatal(err)
	}
	defer os.RemoveAll(testdir)
	db, err := sql.Open("sqlite3", fmt.Sprintf("%s/testdb", testdir))
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	hclient := mockhorizon.New()
	c, err := newCustodian(ctx, db, hclient)
	if err != nil {
		t.Fatal(err)
	}

	go c.pegOutFromExports(ctx)

	var lumen xdr.Asset
	lumen.Type = xdr.AssetTypeAssetTypeNative
	lumenXDR, err := lumen.MarshalBinary()
	if err != nil {
		t.Fatal(err)
	}
	amount := 50
	kp, err := keypair.Random()
	if err != nil {
		t.Fatal(err)
	}
	err = stellar.FundAccount(kp.Address())
	if err != nil {
		t.Fatalf("error funding account %s: %s", kp.Address(), err)
	}

	tempAddr, seqnum, err := SubmitPreExportTx(c.hclient, kp, c.AccountID.Address(), lumen, int64(amount))
	if err != nil {
		t.Fatal(err)
	}

	_, err = c.DB.Exec("INSERT INTO exports (txid, amount, asset_xdr, temp_addr, seqnum, exporter) VALUES ($1, $2, $3, $4, $5, $6)", "", amount, lumenXDR, tempAddr, seqnum, kp.Address())
	if err != nil && err != context.Canceled {
		t.Fatal(err)
	}

	c.exports.Broadcast()

	ch := make(chan struct{})

	go func() {
		var cursor horizon.Cursor
		for {
			err := c.hclient.StreamTransactions(ctx, kp.Address(), &cursor, func(tx horizon.Transaction) {
				log.Printf("received tx: %s", tx.EnvelopeXdr)
				var env xdr.TransactionEnvelope
				err := xdr.SafeUnmarshalBase64(tx.EnvelopeXdr, &env)
				if err != nil {
					t.Fatal(err)
				}
				if env.Tx.SourceAccount.Address() != tempAddr {
					log.Println("source accounts don't match, skipping...")
					return
				}
				if len(env.Tx.Operations) != 2 {
					t.Fatalf("too many operations got %d, want 2", len(env.Tx.Operations))
				}
				op := env.Tx.Operations[0]
				if op.Body.Type != xdr.OperationTypeAccountMerge {
					t.Fatalf("wrong operation type: got %s, want %s", op.Body.Type, xdr.OperationTypeAccountMerge)
				}
				if op.Body.Destination.Address() != kp.Address() {
					t.Fatalf("wrong account merge destination: got %s, want %s", op.Body.Destination.Address(), kp.Address())
				}

				op = env.Tx.Operations[1]
				if op.Body.Type != xdr.OperationTypePayment {
					t.Fatalf("wrong operation type: got %s, want %s", op.Body.Type, xdr.OperationTypePayment)
				}
				paymentOp := op.Body.PaymentOp
				if paymentOp.Destination.Address() != kp.Address() {
					t.Fatalf("incorrect payment destination got %s, want %s", paymentOp.Destination.Address(), kp.Address())
				}
				if paymentOp.Amount != 50 {
					t.Fatalf("got incorrect payment amount %d, want %d", paymentOp.Amount, 50)
				}
				if paymentOp.Asset.Type != xdr.AssetTypeAssetTypeNative {
					t.Fatalf("got incorrect payment asset %s, want lumens", paymentOp.Asset.String())
				}
				close(ch)
			})
			if err != nil {
				log.Printf("error streaming from Horizon: %s, retrying in 1s", err)
				time.Sleep(time.Second)
			}
		}
	}()

	select {
	case <-ctx.Done():
		t.Fatal("context timed out: no peg-out tx seen")
	case <-ch:
	}
}
