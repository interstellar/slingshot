package slidechain

import (
	"bytes"
	"context"
	"encoding/json"
	"math"
	"time"

	"github.com/chain/txvm/crypto/ed25519"
	i10rjson "github.com/chain/txvm/encoding/json"
	"github.com/chain/txvm/errors"
	"github.com/chain/txvm/protocol/bc"
	"github.com/chain/txvm/protocol/txvm"
	"github.com/chain/txvm/protocol/txvm/op"
	"github.com/chain/txvm/protocol/txvm/txvmutil"
	"github.com/stellar/go/xdr"
)

func (c *Custodian) doPostExport(ctx context.Context, assetXDR, anchor, txid []byte, amount, seqnum, peggedOut int64, exporter, temp string, pubkey []byte) error {
	var asset xdr.Asset
	err := asset.UnmarshalBinary(assetXDR)
	if err != nil {
		return errors.Wrap(err, "unmarshaling asset xdr")
	}
	assetBytes, err := asset.MarshalBinary()
	if err != nil {
		return errors.Wrap(err, "marshaling asset bytes")
	}
	assetID := bc.NewHash(txvm.AssetID(importIssuanceSeed[:], assetBytes))
	ref := struct {
		AssetXDR []byte `json:"asset"`
		Temp     string `json:"temp"`
		Seqnum   int64  `json:"seqnum"`
		Exporter string `json:"exporter"`
		Amount   int64  `json:"amount"`
		Anchor   []byte `json:"anchor"`
		Pubkey   []byte `json:"pubkey"`
	}{
		assetXDR,
		temp,
		seqnum,
		exporter,
		amount,
		anchor,
		pubkey,
	}
	refdata, err := json.Marshal(ref)
	if err != nil {
		return errors.Wrap(err, "marshaling reference data")
	}
	refdataHex := i10rjson.HexBytes(refdata)
	b := new(txvmutil.Builder)
	b.Tuple(func(contract *txvmutil.TupleBuilder) { // {'C', ...}
		contract.PushdataByte(txvm.ContractCode)
		contract.PushdataBytes(exportContract1Seed[:])
		contract.PushdataBytes(exportContract2Prog)
		contract.Tuple(func(tup *txvmutil.TupleBuilder) { // {'T', pubkey}
			tup.PushdataByte(txvm.TupleCode)
			tup.Tuple(func(pktup *txvmutil.TupleBuilder) {
				pktup.PushdataBytes(pubkey)
			})
		})
		contract.Tuple(func(tup *txvmutil.TupleBuilder) { // {'V', amount, assetID, anchor}
			tup.PushdataByte(txvm.ValueCode)
			tup.PushdataInt64(amount)
			tup.PushdataBytes(assetID.Bytes())
			tup.PushdataBytes(anchor)
		})
		contract.Tuple(func(tup *txvmutil.TupleBuilder) { // {'S', refdata}
			tup.PushdataByte(txvm.BytesCode)
			tup.PushdataBytes(refdataHex)
		})
	})
	b.PushdataInt64(peggedOut).Op(op.Put)                               // con stack: snapshot; arg stack: selector
	b.Op(op.Input).Op(op.Call)                                          // arg stack: sigchecker
	b.PushdataBytes(c.InitBlockHash.Bytes())                            // con stack: blockid; arg stack: sigchecker
	b.PushdataInt64(int64(bc.Millis(time.Now().Add(10 * time.Minute)))) // con stack: blockid, expmss; arg stack: sigchecker
	b.Op(op.Nonce).Op(op.Finalize)                                      // arg stack: sigchecker

	prog1 := b.Build()
	vm, err := txvm.Validate(prog1, 3, math.MaxInt64, txvm.StopAfterFinalize)
	if err != nil {
		return errors.Wrap(err, "computing transaction ID")
	}
	sig := ed25519.Sign(c.privkey, vm.TxID[:])
	b.Op(op.Get).PushdataBytes(sig).Op(op.Put) // con stack: sigchecker; arg stack: sig
	b.Op(op.Call)

	prog2 := b.Build()
	tx, err := bc.NewTx(prog2, 3, math.MaxInt64)
	if err != nil {
		return errors.Wrap(err, "making post-export tx")
	}
	if err != nil {
		return errors.Wrap(err, "building post-export tx")
	}
	r, err := c.S.submitTx(ctx, tx)
	if err != nil {
		return errors.Wrap(err, "submitting post-export tx")
	}
	err = c.S.waitOnTx(ctx, tx.ID, r)
	if err != nil {
		return errors.Wrap(err, "waiting on post-export tx to hit txvm")
	}
	_, err = c.DB.ExecContext(ctx, `DELETE FROM exports WHERE txid=$1`, txid)
	return errors.Wrapf(err, "deleting export for tx %x", txid)
}

// IsPostExportTx returns whether or not a txvm transaction matches the slidechain post-export tx format.
//
// Expected log is
// {"I", ...}
// {"X", ...}
// {"L", ...}
// {"N", ...}
// {"R", ...}
// {"F", ...}
func IsPostExportTx(tx *bc.Tx, asset xdr.Asset, amount int64, temp, exporter string, seqnum int64, anchor, pubkey []byte) bool {
	if len(tx.Log) != 6 {
		return false
	}
	if tx.Log[0][0].(txvm.Bytes)[0] != txvm.InputCode {
		return false
	}
	if tx.Log[1][0].(txvm.Bytes)[0] != txvm.RetireCode {
		return false
	}
	if tx.Log[2][0].(txvm.Bytes)[0] != txvm.LogCode {
		return false
	}
	if tx.Log[3][0].(txvm.Bytes)[0] != txvm.NonceCode {
		return false
	}
	if tx.Log[4][0].(txvm.Bytes)[0] != txvm.TimerangeCode {
		return false
	}
	if tx.Log[5][0].(txvm.Bytes)[0] != txvm.FinalizeCode {
		return false
	}
	assetXDR, err := xdr.MarshalBase64(asset)
	if err != nil {
		return false
	}
	ref := struct {
		AssetXDR string `json:"asset"`
		Temp     string `json:"temp"`
		Seqnum   int64  `json:"seqnum"`
		Exporter string `json:"exporter"`
		Amount   int64  `json:"amount"`
		Anchor   []byte `json:"anchor"`
		Pubkey   []byte `json:"pubkey"`
	}{
		assetXDR,
		temp,
		seqnum,
		exporter,
		amount,
		anchor,
		pubkey,
	}
	refdata, err := json.Marshal(ref)
	if !bytes.Equal(refdata, tx.Log[2][2].(txvm.Bytes)) {
		return false
	}
	return true
}
