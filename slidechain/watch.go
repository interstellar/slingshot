package slidechain

import (
	"context"
	"database/sql"
	"encoding/json"
	"log"
	"time"

	"github.com/bobg/sqlutil"
	i10rjson "github.com/chain/txvm/encoding/json"
	"github.com/chain/txvm/protocol/bc"
	"github.com/chain/txvm/protocol/txvm"
	i10rnet "github.com/interstellar/starlight/net"
	"github.com/stellar/go/clients/horizon"
	"github.com/stellar/go/xdr"
)

// Runs as a goroutine until ctx is canceled.
func (c *Custodian) watchPegs(ctx context.Context) {
	defer log.Println("watchPegs exiting")
	backoff := i10rnet.Backoff{Base: 100 * time.Millisecond}

	var cur horizon.Cursor
	err := c.DB.QueryRow("SELECT cursor FROM custodian").Scan(&cur)
	if err != nil && err != sql.ErrNoRows {
		log.Fatal(err)
	}

	for {
		err := c.hclient.StreamTransactions(ctx, c.AccountID.Address(), &cur, func(tx horizon.Transaction) {
			log.Printf("handling Stellar tx %s", tx.ID)

			var env xdr.TransactionEnvelope
			err := xdr.SafeUnmarshalBase64(tx.EnvelopeXdr, &env)
			if err != nil {
				log.Fatal("error unmarshaling Stellar tx: ", err)
			}

			if env.Tx.Memo.Type != xdr.MemoTypeMemoHash {
				return
			}

			nonceHash := (*env.Tx.Memo.Hash)[:]
			for _, op := range env.Tx.Operations {
				if op.Body.Type != xdr.OperationTypePayment {
					continue
				}
				payment := op.Body.PaymentOp
				if !payment.Destination.Equals(c.AccountID) {
					continue
				}

				// This operation is a payment to the custodian's account - i.e., a peg.
				// We update the db to note that we saw this entry on the Stellar network.
				// We also populate the amount and asset_xdr with the values in the Stellar tx.
				assetXDR, err := payment.Asset.MarshalBinary()
				if err != nil {
					log.Fatalf("marshaling asset xdr: %s", err)
					return
				}
				resulted, err := c.DB.ExecContext(ctx, `UPDATE pegs SET amount=$1, asset_xdr=$2, stellar_tx=1 WHERE nonce_hash=$3 AND stellar_tx=0`, payment.Amount, assetXDR, nonceHash)
				if err != nil {
					log.Fatalf("updating stellar_tx=1 for hash %x: %s", nonceHash, err)
				}

				// We confirm that only a single row was affected by the update query.
				numAffected, err := resulted.RowsAffected()
				if err != nil {
					log.Fatalf("checking rows affected by update query for hash %x: %s", nonceHash, err)
				}
				if numAffected != 1 {
					log.Fatalf("multiple rows affected by update query for hash %x", nonceHash)
				}

				// We update the cursor to avoid double-processing a transaction.
				_, err = c.DB.ExecContext(ctx, `UPDATE custodian SET cursor=$1 WHERE seed=$2`, tx.PT, c.seed)
				if err != nil {
					log.Fatalf("updating cursor: %s", err)
					return
				}

				// Wake up a goroutine that executes imports for not-yet-imported pegs.
				log.Printf("broadcasting import for tx with nonce hash %x", nonceHash)
				c.imports.Broadcast()
			}
		})
		if err == context.Canceled {
			return
		}
		if err != nil {
			log.Printf("error streaming from horizon: %s, retrying...", err)
		}
		ch := make(chan struct{})
		go func() {
			time.Sleep(backoff.Next())
			close(ch)
		}()
		select {
		case <-ctx.Done():
			return
		case <-ch:
		}
	}
}

// Runs as a goroutine.
func (c *Custodian) watchExports(ctx context.Context) {
	defer log.Println("watchExports exiting")
	r := c.S.w.Reader()
	for {
		got, ok := r.Read(ctx)
		if !ok {
			if ctx.Err() == context.Canceled {
				return
			}
			log.Fatal("error reading block from multichan")
		}
		b := got.(*bc.Block)
		for _, tx := range b.Transactions {
			// Check if the transaction has either expected length for an export tx.
			// Confirm that its input, log, and output entries are as expected.
			// If so, look for a specially formatted log ("L") entry
			// that specifies the Stellar asset code to peg out and the Stellar recipient account ID.
			if len(tx.Log) != 4 && len(tx.Log) != 6 {
				continue
			}
			if tx.Log[0][0].(txvm.Bytes)[0] != txvm.InputCode {
				continue
			}
			if tx.Log[1][0].(txvm.Bytes)[0] != txvm.LogCode {
				continue
			}

			outputIndex := len(tx.Log) - 2
			if tx.Log[outputIndex][0].(txvm.Bytes)[0] != txvm.OutputCode {
				continue
			}

			logItem := tx.Log[1]
			var info struct {
				AssetXDR []byte `json:"asset"`
				Temp     string `json:"temp"`
				Seqnum   int64  `json:"seqnum"`
				Exporter string `json:"exporter"`
				Amount   int64  `json:"amount"`
				Anchor   []byte `json:"anchor"`
				Pubkey   []byte `json:"pubkey"`
			}
			err := json.Unmarshal(i10rjson.HexBytes(logItem[2].(txvm.Bytes)), &info)
			if err != nil {
				continue
			}
			exportedAssetBytes := txvm.AssetID(importIssuanceSeed[:], info.AssetXDR)

			// Record the export in the db,
			// then wake up a goroutine that executes peg-outs on the main chain.
			const q = `
				INSERT INTO exports 
				(txid, exporter, amount, asset_xdr, temp, seqnum, anchor, pubkey)
				VALUES ($1, $2, $3, $4, $5, $6, $7, $8)`
			_, err = c.DB.ExecContext(ctx, q, tx.ID.Bytes(), info.Exporter, info.Amount, info.AssetXDR, info.Temp, info.Seqnum, info.Anchor, info.Pubkey)
			if err != nil {
				log.Fatalf("recording export tx: %s", err)
			}

			log.Printf("recorded export: %d of txvm asset %x (Stellar %x) for %s", info.Amount, exportedAssetBytes, info.AssetXDR, info.Exporter)

			c.exports.Broadcast()
		}
	}
}

// Runs as a goroutine
func (c *Custodian) watchPegOuts(ctx context.Context) {
	defer log.Print("watchPegOuts exiting")

	// Poll the database every minute for unprocessed exports.
	go func(ctx context.Context) {
		defer log.Print("export table poll exiting")
		for t := time.Tick(1 * time.Minute); ; {
			const q = `SELECT amount, asset_xdr, exporter, temp, seqnum, anchor, pubkey FROM exports WHERE (pegged_out=1 OR pegged_out=3)`
			var (
				txids, assetXDRs, anchors, pubkeys [][]byte
				amounts, seqnums, peggedOuts       []int64
				exporters, temps                   []string
			)
			err := sqlutil.ForQueryRows(ctx, c.DB, q, func(txid []byte, amount int64, assetXDR []byte, exporter, temp string, seqnum, peggedOut int64, anchor, pubkey []byte) {
				txids = append(txids, txid)
				amounts = append(amounts, amount)
				assetXDRs = append(assetXDRs, assetXDR)
				exporters = append(exporters, exporter)
				temps = append(temps, temp)
				seqnums = append(seqnums, seqnum)
				peggedOuts = append(peggedOuts, peggedOut)
				anchors = append(anchors, anchor)
				pubkeys = append(pubkeys, pubkey)
			})
			if err == context.Canceled {
				return
			}
			if err != nil {
				log.Fatalf("querying peg-outs: %s", err)
			}
			for i, txid := range txids {
				err = c.doPostExport(ctx, assetXDRs[i], anchors[i], txid, amounts[i], seqnums[i], peggedOuts[i], exporters[i], temps[i], pubkeys[i])
				if err != nil {
					if err == context.Canceled {
						return
					}
					log.Fatal(err)
				}
			}
			select {
			case <-t:
				continue
			case <-ctx.Done():
				return
			}
		}
	}(ctx)

	for {
		// Read in new peg-outs via channel.
		var pegOut PegOut
		select {
		case <-ctx.Done():
			return
		case pegOut = <-c.pegouts:
		}
		const q = `SELECT amount, asset_xdr, exporter, temp, seqnum, anchor, pubkey FROM exports WHERE txid=$1 AND pegged_out=$2`
		var (
			assetXDR, anchor, pubkey []byte
			amount, seqnum           int64
			exporter, temp           string
		)
		err := sqlutil.ForQueryRows(ctx, c.DB, q, pegOut.txid, pegOut.state, func(qAmount int64, qAssetXDR []byte, qExporter, qTemp string, qSeqnum int64, qAnchor, qPubkey []byte) {
			assetXDR, anchor, pubkey = qAssetXDR, qAnchor, qPubkey
			amount, seqnum = qAmount, qSeqnum
			exporter, temp = qExporter, qTemp
		})
		if err == context.Canceled {
			return
		}
		if err != nil {
			log.Fatalf("querying peg-outs: %s", err)
		}
		err = c.doPostExport(ctx, assetXDR, anchor, pegOut.txid, amount, seqnum, int64(pegOut.state), exporter, temp, pubkey)
		if err != nil {
			if err == context.Canceled {
				return
			}
			log.Fatal(err)
		}
	}
}
