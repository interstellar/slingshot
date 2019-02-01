package slidechain

import (
	"context"
	"io/ioutil"
	"log"
	"net/http"
	"strconv"
	"sync"
	"time"

	"github.com/bobg/multichan"
	"github.com/chain/txvm/errors"
	"github.com/chain/txvm/protocol"
	"github.com/chain/txvm/protocol/bc"
	"github.com/golang/protobuf/proto"
	"github.com/interstellar/slingshot/slidechain/net"
)

// TODO: make this configurable.
var blockInterval = 5 * time.Second

type submitter struct {
	// Protects bb.
	bbmu sync.Mutex

	// Normally nil. Once a tx is submitted, this is set to a new block
	// builder and a timer set. Other txs that arrive during that
	// interval are added to the block a-building. When the timer fires,
	// the block is added to the blockchain and this field is set back to nil.
	//
	// This is the only way that blocks are added to the chain.
	bb *protocol.BlockBuilder

	// New blocks are written here.
	// Anything monitoring the blockchain can create a reader and consume them.
	// (Really, what we want here is the Sequence "pin" mechanism.)
	w *multichan.W

	initialBlock *bc.Block

	chain *protocol.Chain
}

func (s *submitter) submitTx(ctx context.Context, tx *bc.Tx, wait bool) error {
	s.bbmu.Lock()
	defer s.bbmu.Unlock()

	if s.bb == nil {
		s.bb = protocol.NewBlockBuilder()
		nextBlockTime := time.Now().Add(blockInterval)

		st := s.chain.State()
		if st.Header == nil {
			err := st.ApplyBlockHeader(s.initialBlock.BlockHeader)
			if err != nil {
				return errors.Wrap(err, "initializing empty state")
			}
		}

		err := s.bb.Start(s.chain.State(), bc.Millis(nextBlockTime))
		if err != nil {
			return errors.Wrap(err, "starting a new tx pool")
		}
		log.Printf("starting new block, will commit at %s", nextBlockTime)
		time.AfterFunc(blockInterval, func() {
			s.bbmu.Lock()
			defer s.bbmu.Unlock()

			defer func() { s.bb = nil }()

			unsignedBlock, newSnapshot, err := s.bb.Build()
			if err != nil {
				log.Fatalf("building new block: %s", err)
			}
			if len(unsignedBlock.Transactions) == 0 {
				log.Print("skipping commit of empty block")
				return
			}
			b := &bc.Block{UnsignedBlock: unsignedBlock}
			err = s.chain.CommitAppliedBlock(ctx, b, newSnapshot)
			if err != nil {
				log.Fatalf("committing new block: %s", err)
			}

			s.w.Write(b)
			log.Printf("committed block %d with %d transaction(s)", unsignedBlock.Height, len(unsignedBlock.Transactions))
		})
	}

	err := s.bb.AddTx(bc.NewCommitmentsTx(tx))
	if err != nil {
		return errors.Wrap(err, "adding tx to pool")
	}
	log.Printf("added tx %x to the pending block", tx.ID.Bytes())
	if wait {
		log.Printf("waiting on tx %x to hit txvm", tx.ID.Bytes())
		r := s.w.Reader()
		for {
			got, ok := r.Read(ctx)
			if !ok {
				log.Printf("error reading block from multichan while waiting for tx %x to hit txvm", tx.ID.Bytes())
				return ctx.Err()
			}
			b := got.(*bc.Block)
			for _, gotTx := range b.Transactions {
				if gotTx.ID == tx.ID {
					log.Printf("tx %x hit txvm chain", tx.ID.Bytes())
					return nil
				}
			}
		}
	}
	return nil
}

func (s *submitter) ServeHTTP(w http.ResponseWriter, req *http.Request) {
	ctx := req.Context()

	wantStr := req.FormValue("wait")
	var (
		wantInt uint64
		err     error
	)
	if wantStr != "" {
		wantInt, err = strconv.ParseUint(wantStr, 10, 64)
		if err != nil {
			net.Errorf(w, http.StatusBadRequest, "parsing wait: %s", err)
			return
		}
		if wantInt != 1 {
			net.Errorf(w, http.StatusBadRequest, "wait can only be 1")
			return
		}
	}
	wait := (wantInt != 0)

	bits, err := ioutil.ReadAll(req.Body)
	if err != nil {
		net.Errorf(w, http.StatusInternalServerError, "reading request body: %s", err)
		return
	}

	var rawTx bc.RawTx
	err = proto.Unmarshal(bits, &rawTx)
	if err != nil {
		net.Errorf(w, http.StatusBadRequest, "parsing request body: %s", err)
		return
	}

	tx, err := bc.NewTx(rawTx.Program, rawTx.Version, rawTx.Runlimit)
	if err != nil {
		net.Errorf(w, http.StatusBadRequest, "building tx: %s", err)
		return
	}

	err = s.submitTx(ctx, tx, wait)
	if err != nil {
		net.Errorf(w, http.StatusBadRequest, "submitting tx: %s", err)
		return
	}
	w.WriteHeader(http.StatusNoContent)
}

func (s *submitter) Get(w http.ResponseWriter, req *http.Request) {
	wantStr := req.FormValue("height")
	var (
		want uint64 = 1
		err  error
	)
	if wantStr != "" {
		want, err = strconv.ParseUint(wantStr, 10, 64)
		if err != nil {
			net.Errorf(w, http.StatusBadRequest, "parsing height: %s", err)
			return
		}
	}

	height := s.chain.Height()
	if want == 0 {
		want = height
	}
	if want > height {
		ctx := req.Context()
		waiter := s.chain.BlockWaiter(want)
		select {
		case <-waiter:
			// ok
		case <-ctx.Done():
			net.Errorf(w, http.StatusRequestTimeout, "timed out")
			return
		}
	}

	ctx := req.Context()

	b, err := s.chain.GetBlock(ctx, want)
	if err != nil {
		net.Errorf(w, http.StatusInternalServerError, "getting block %d: %s", want, err)
		return
	}

	bits, err := b.Bytes()
	if err != nil {
		net.Errorf(w, http.StatusInternalServerError, "serializing block %d: %s", want, err)
		return
	}

	w.Header().Set("Content-Type", "application/octet-stream")
	_, err = w.Write(bits)
	if err != nil {
		net.Errorf(w, http.StatusInternalServerError, "sending response: %s", err)
		return
	}
}
