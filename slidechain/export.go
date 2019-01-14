package main

import (
	"context"

	"github.com/bobg/sqlutil"
	"github.com/chain/txvm/errors"
	b "github.com/stellar/go/build"
	"github.com/stellar/go/xdr"
)

const baseFee = 10

func (c *custodian) pegOutFromExports(ctx context.Context) error {
	c.exports.L.Lock()
	defer c.exports.L.Unlock()
	for {
		c.exports.Wait()
		const q = `SELECT txid, recipient, amount, asset_xdr FROM exports WHERE exported=0`
		err := sqlutil.ForQueryRows(ctx, c.db, q, func(txid, recipient string, amount int, assetXDR []byte) error {
			var recipientID xdr.AccountId
			err := recipientID.SetAddress(recipient)
			if err != nil {
				return errors.Wrapf(err, "setting recipient account ID %s", recipient)
			}
			var asset xdr.Asset
			err = xdr.SafeUnmarshal(assetXDR, &asset)
			if err != nil {
				return errors.Wrapf(err, "unmarshalling asset XDR from asset %s", asset.String())
			}
			// TODO(vniu): flag txs that fail with unretriable errors in the db
			err = c.pegOut(ctx, recipientID, asset, amount)
			if err != nil {
				return err
			}
			_, err = c.db.ExecContext(ctx, `UPDATE exports SET exported=1 WHERE txid=$1`, txid)
			return err
		})
		if err != nil {
			return err
		}
	}
}

func (c *custodian) pegOut(ctx context.Context, recipient xdr.AccountId, asset xdr.Asset, amount int) error {
	tx, err := c.buildPegOutTx(recipient, asset, amount)
	txenv, err := tx.Sign(c.seed)
	if err != nil {
		return errors.Wrap(err, "signing tx")
	}
	txstr, err := xdr.MarshalBase64(txenv.E)
	if err != nil {
		return errors.Wrap(err, "marshaling tx to base64")
	}
	_, err = c.hclient.SubmitTransaction(txstr)
	return errors.Wrap(err, "submitting tx")
}

func (c *custodian) buildPegOutTx(recipient xdr.AccountId, asset xdr.Asset, amount int) (*b.TransactionBuilder, error) {
	var paymentOp b.PaymentBuilder
	switch asset.Type {
	case xdr.AssetTypeAssetTypeNative:
		paymentOp = b.Payment(
			b.Destination{AddressOrSeed: recipient.Address()},
			b.NativeAmount{Amount: string(amount)},
		)
	case xdr.AssetTypeAssetTypeCreditAlphanum4:
		paymentOp = b.Payment(
			b.Destination{AddressOrSeed: recipient.Address()},
			b.CreditAmount{
				Code:   string(asset.AlphaNum4.AssetCode[:]),
				Issuer: asset.AlphaNum4.Issuer.Address(),
				Amount: string(amount),
			},
		)
	case xdr.AssetTypeAssetTypeCreditAlphanum12:
		paymentOp = b.Payment(
			b.Destination{AddressOrSeed: recipient.Address()},
			b.CreditAmount{
				Code:   string(asset.AlphaNum12.AssetCode[:]),
				Issuer: asset.AlphaNum12.Issuer.Address(),
				Amount: string(amount),
			},
		)
	}
	return b.Transaction(
		b.Network{Passphrase: c.network},
		b.SourceAccount{AddressOrSeed: c.accountID.Address()},
		b.AutoSequence{SequenceProvider: c.hclient},
		b.BaseFee{Amount: baseFee},
		paymentOp,
	)
}
