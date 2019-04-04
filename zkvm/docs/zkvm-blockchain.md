# ZkVM blockchain specification

This is the specification for the ZkVM blockchain,
a blockchain containing
[ZkVM transactions](zkvm-spec.md).

Nodes participating in a ZkVM blockchain network must implement the data types and perform the procedures described in this document.
Specifically:

- Each node maintains a [blockchain state](#blockchain-state).
- A node creating a new ZkVM network performs the [start new network](#start-new-network) procedure.
- A node joining an existing ZkVM network performs the [join existing network](#join-existing-network) procedure.
- Each node performs the [apply block](#apply-block) procedure on the arrival of each new block.

This document does not describe the mechanism for producing blocks from pending transactions,
nor for choosing among competing blocks to include in the authoritative chain
(a.k.a. consensus).

_TBD: add a consensus spec._

# Data types

## Blockchain state

The state of a ZkVM blockchain is given by the blockchain-state structure.
Each node maintains a copy of this structure.
As each new [block](#block) arrives,
it is [applied](#apply-block) to the current state to produce a new,
updated state.

The blockchain state contains:

- `initialheader`: The initial block header
  (the header with height 1).
  This never changes.
- `tipheader`: The latest block header.
- `utxos`: The set of current [utxo IDs](zkvm-spec.md#utxo).
- `nonces`: The set of `<anchor,maxtime_ms>` pairs for all current [nonces](zkvm-spec.md#nonce).
- `refids`: The set of recent [block IDs](#block-id).
  The size of this set is bounded by `block.header.refscount`.
  Block IDs in this set may be referenced by nonces.

## Block

A block contains:

- `header`: A [block header](#block-header).
- `txs`: A list of [transactions](zkvm-spec.md#transaction).

The initial block
(at height 1)
has an empty list of transactions.

## Block header

A block header contains:

- `version`: Integer version number,
  set to 1.
- `height`: Integer block height.
  Initial block has height 1.
  Height increases by 1 with each new block.
- `previd`: ID of the preceding block.
  For the initial block
  (which has no predecessor),
  this is an all-zero string of 32 bytes.
- `timestamp_ms`: Integer timestamp of the block in milliseconds since the Unix epoch:
  00:00:00 UTC Jan 1, 1970.
  Each new block must have a time strictly later than the block before it.
- `txroot`: 32-byte [Merkle root hash](zkvm-spec.md#merkle-binary-tree) of the transactions in the block.
- `utxoroot`: 32-byte [Merkle patricia root hash](#merkle-patricia-tree) of the utxo set after applying all transactions in the block.
- `nonceroot`: 32-byte [Merkle patricia root hash](#merkle-patricia-tree) of the nonce set after applying all transactions in the block.
- `refscount`: Integer number of recent block IDs to store for reference.
  A new block may specify a lower `refscount` than its predecessor but may not increase it by more than 1.
- `ext`: Variable-length byte string to contain future extensions.
  Empty in version 1.

## Block ID

A block ID is computed from a [block header](#block-header) using the [transcript](zkvm-spec.md#transcript) mechanism:

```
T = Transcript("ZkVM.blockheader")
T.commit("version", LE64(version))
T.commit("height", LE64(height))
T.commit("previd", previd)
T.commit("timestamp_ms", LE64(timestamp_ms))
T.commit("txroot", txroot)
T.commit("utxoroot", utxoroot)
T.commit("nonceroot", nonceroot)
T.commit("refscount", LE64(refscount))
T.commit("ext", ext)
blockid = T.challenge_bytes("id")
```

## Merkle patricia tree

A Merkle patricia tree is similar to a [Merkle binary tree](zkvm-spec.md#merkle-binary-tree).
Its membership uniquely determines its shape.
Each node hashes the subtrees beneath it.
The root node’s hash is a commitment to the full membership of the tree.
It is possible to create and verify compact proofs of membership.

Unlike a Merkle binary tree,
a Merkle patricia tree is a radix tree
(in which subtrees of a given node share a common prefix)
with variable-length branches that allow for efficient updates.
It is therefore preferable to a Merkle binary tree for large sets with frequent and comparatively small updates,
specifically the utxo set and the nonce set.

As with the Merkle binary tree,
we define a Merkle patricia tree in terms of [transcripts](zkvm-spec.md#transcript).
Leaves and nodes in the tree use the same instance of a transcript:

```
T = Transcript(<label>)
```

(where `<label>` is specified by the calling protocol).

The input to the *Merkle patricia tree hash*
(MPTH)
is a list of data entries.
These entries will be hashed to form the leaves of the merkle hash tree.
The output is a single 32-byte hash value.
The input list must be prefix-free;
that is,
no element can be a prefix of any other.
Given a sorted list of n unique inputs,

```
D[n] = {d(0), d(1), ..., d(n-1)}
```

the MPTH is thus defined as follows.

The hash of an empty Merkle patricia tree list is a 32-byte challenge string with the label `patricia.empty`:

```
MPTH(T, {}) = T.challenge_bytes("patricia.empty")
```

To compute the hash of a list with one entry,
commit it to the transcript with the label `patricia.leaf` and then generate a 32-byte challenge string with the same label:

```
T.commit("patricia.leaf", d(0))
MPTH(T, {d{0)}) = T.challenge_bytes("patricia.leaf")
```

To compute the hash of a list with two or more entries:
1. Let the bit string `p` be the longest common prefix of all entries;
2. Let k be the number of items with prefix `p||0`
   (that is,
   `p` concatenated with the single bit 0).
3. Let L be recursively defined as `MPTH(T, D[0:k])`
   (the hash of the first `k` elements of D).
4. Commit `L` to `T` with the label `patricia.left`.
5. Let R be recursively defined as `MPTH(T, D[k:n])`
   (the hash of the remaining `n-k` elements of D).
5. Commit `R` to `T` with the label `patricia.right`.
6. Generate a 32-byte challenge string with the label `patricia.node`.

```
T.commit("patricia.left", MPTH(T, D[0:k]))
T.commit("patricia.right", MPTH(T, D[k:n]))
MPTH(T, D) = T.challenge_bytes("patricia.node")
```


# Procedures

In the descriptions that follow,
the word “verify” means to test whether a condition is true.
If it’s false,
all pending procedures abort and a failure result is returned.

## Start new network

A node starts here when creating a new blockchain network.
Its [blockchain state](#blockchain-state) is set to the result of the procedure.

Inputs:
- `timestamp_ms`,
  the current time as a number of milliseconds since the Unix epoch:
  00:00:00 UTC Jan 1,
  1970.
- `refscount`,
  the number of recent block ids to cache for
  [nonce](zkvm-spec.md#nonce)
  uniqueness.

Output:
- Blockchain state.

Procedure:
1. [Make an initial block header](#make-initial-block-header) `initialheader` from `timestamp_ms` and `refscount`.
2. Return a blockchain state with its fields set as follows:
   - `initialheader`: `initialheader`
   - `tipheader`: `initialheader`
   - `utxos`: empty set
   - `nonces`: empty set
   - `refids`: empty set

## Make initial block header

Inputs:
- `timestamp_ms`,
  the current time as a number of milliseconds since the Unix epoch:
  00:00:00 UTC Jan 1, 1970.
- `refscount`,
  the number of recent block ids to cache for [nonce](zkvm-spec.md#nonce) uniqueness.

Output:
- A [block header](#block-header).

Procedure:
1. [Compute txroot](#compute-txroot) from an empty list of transaction ids.
2. [Compute utxoroot](#compute-utxoroot) from an empty set of utxos.
3. [Compute nonceroot](#compute-nonceroot) from an empty set of nonce anchors.
4. Return a block header with its fields set as follows:
   - `version`: 1
   - `height`: 1
   - `previd`: all-zero string of 32-bytes
   - `timestamp_ms`: `timestamp_ms`
   - `txroot`: `txroot`
   - `utxoroot`: `utxoroot`
   - `nonceroot`: `nonceroot`
   - `refscount`: `refscount`
   - `ext`: empty

## Join existing network

A new node starts here when joining a running network.
It must either:
- obtain all historical blocks,
  [applying](#apply-block) them one by one to reproduce the latest [blockchain state](#blockchain-state);
  or
- obtain a recent copy of the blockchain state `state` from a trusted source
  (e.g., another node that has already validated the full history of the blockchain)
  and begin applying blocks beginning at `state.tipheader.height+1`.

An obtained (as opposed to computed) blockchain state `state` may be partially validated by [computing the utxoroot](#compute-utxoroot) from `state.utxos` and verifying that it equals `state.header.utxoroot`.


## Validate block

Validating a block checks it for correctness outside the context of a particular [blockchain state](#blockchain-state).

Additional correctness checks against a particular blockchain state happen during the [apply block](#apply-block) procedure,
of which this is a subroutine.

Inputs:
- `block`,
  the block to validate,
  at height 2 or above.
- `prevheader`,
  the header of the previous block.

Output:
- list of [transaction logs](zkvm-spec.md#transaction-log),
  one for each transaction in block.txs.

Procedure:
1. Verify `block.header.version >= prevheader.version`.
2. If `block.header.version == 1`, verify `block.header.ext` is empty.
3. Verify `block.header.height == prevheader.height+1`.
4. Verify `block.header.previd` equals the [block ID](#block-id) of `prevheader`.
5. Verify `block.header.timestamp_ms > prevheader.timestamp_ms`.
6. Verify `block.header.refscount >= 0` and `block.header.refscount <= prevheader.refscount + 1`.
7. Let `txlogs` and `txids` be the result of [executing the transactions in block.txs](#execute-transaction-list) with `block.header.version` and `block.header.timestamp_ms`.
8. [Compute txroot](#compute-txroot) from `txids`.
9. Verify `txroot == block.header.txroot`.
10. Return `txlogs`.


## Make block

Inputs:
- `state`,
  a [blockchain state](#blockchain-state).
- `version`,
  a version number for the new block.
  Note that this must be equal to or greater than `state.tipheader.version`,
  the version number of the previous block header.
- `timestamp_ms`,
  a time for the new block as milliseconds since the Unix epoch,
  00:00:00 UTC Jan 1, 1970.
  This must be strictly greater than `state.tipheader.timestamp_ms`,
  the timestamp of the previous block header.
- `refscount`,
  a number of recent block IDs to store for reference.
  Note that this must lie between 0 and `state.tipheader.refscount+1`,
  inclusive.
- `txs`,
  a list of [transactions](zkvm-spec.md#transaction).
- `ext`,
  the contents of the new block’s “extension” field.
  Note that at this writing,
  only block version 1 is defined,
  which requires `ext` to be empty.

Output:
- a new [block](#block) containing `txs`.

Procedure:
1. Let `previd` be the [block ID](#block-id) of `state.tipheader`.
2. Let `txlogs` and `txids` be the result of [executing txs](#execute-transaction-list) with `version` and `timestamp_ms`.
3. Let `state´` be the result of [applying txlogs](#apply-transaction-list) to `state`.
4. Let `txids` be the list of [transaction IDs](zkvm-spec.md#transaction-id) of the transactions in `txs`,
   computed from each transaction’s [header entry](zkvm-spec.md#header-entry) and the corresponding item from `txlogs`.
5. [Compute txroot](#compute-txroot) from `txids` to produce `txroot`.
6. [Compute utxoroot](#compute-utxoroot) from `state′.utxos` to produce `utxoroot`.
7. [Compute nonceroot](#compute-nonceroot) from the anchors in `state′.nonces` to produce `nonceroot`.
8. Let `h` be a [block header](#block-header) with its fields set as follows:
   - `version`: `version`
   - `height`: `state.tipheader.height+1`
   - `previd`: `previd`
   - `timestamp_ms`: `timestamp_ms`
   - `txroot`: `txroot`
   - `utxoroot`: `utxoroot`
   - `nonceroot`: `nonceroot`
   - `refscount`: `refscount`
   - `ext`: `ext`
9. Return a block with header `h` and transactions `txs`.


## Execute transaction list

Input:
- `txs`,
  a list of [transactions](zkvm-spec.md#transaction).
- `version`,
  a version number for a block.
- `timestamp_ms`,
  a block timestamp as milliseconds since the Unix epoch,
  00:00:00 UTC Jan 1, 1970.

Outputs:
- a list of [transaction logs](zkvm-spec.md#transaction-log),
  one per transaction in `txs`.
- a list of [transaction IDs](zkvm-spec.md#transaction-id),
  one per transaction in `txs`.

Procedure:
1. Let `txlogs` be an empty list of transaction logs.
   Let `txids` be an empty list of transaction IDs.
2. For each transaction `tx` in `txs`:
   1. Verify `tx.mintime_ms <= timestamp_ms <= tx.maxtime_ms`.
   2. If `version == 1`,
      verify `tx.version == 1`.
   3. [Execute](zkvm-spec.md#vm-execution) `tx` to produce transaction log `txlog`.
   4. Add `txlog` to `txlogs`.
   5. Compute transaction ID `txid` from the [header entry](zkvm-spec.md#header-entry) of `tx` and from `txlog`.
   6. Add `txid` to `txids`.
3. Return `txlogs` and `txids`.

Note that step 2 can be parallelized across `txs`.


## Apply block

Applying a block causes a node to replace its [blockchain state](#blockchain-state) with the updated state that results.

Inputs:
- `block`,
  the [block](#block) to apply.
- `state`,
  the current blockchain state.

Output:
- New blockchain state `state′`.

Procedure:
1. Let `txlogs` be the result of [validating](#validate-block) `block` with `prevheader` set to `state.tipheader`.
2. Let `state′` be `state`.
3. Remove items from `state′.nonces` where `maxtime_ms < block.header.timestamp_ms`.
4. Let `state′′` be the result of [applying txlogs](#apply-transaction-list) to `state′`.
5. Set `state′ <- state′′`.
6. [Compute utxoroot](#compute-utxoroot) from `state′.utxos`.
7. Verify `block.header.utxoroot == utxoroot`.
8. [Compute nonceroot](#compute-nonceroot) from the anchors in `state′.nonces`.
9. Verify `block.header.nonceroot == nonceroot`.
10. Set `state′.tipheader <- block.header`.
11. Add `block.header` to the end of the `state′.refids` list.
12. Prune `state′.refids` to the number of items specified by `block.header.refscount` by removing the oldest IDs.
13. Return `state′`.


## Apply transaction list

Inputs:
- `state`,
  a [blockchain state](#blockchain-state).
- `txlogs`,
  a list of [transaction logs](zkvm-spec.md#transaction-log).

Output:
- Updated blockchain state.

Procedure:
1. Let `state′` be `state`.
2. For each `txlog` in `txlogs`,
   in order:
   1. Let `state′′` be the result of [applying the txlog](#apply-transaction-log) to `state′` to produce `state′′`.
   2. Set `state′` <- `state′′`.
3. Return `state′`.


## Apply transaction log

Inputs:
- `txlog`,
  a [transaction log](zkvm-spec.md#transaction-log).
- `state`,
  a [blockchain state](#blockchain-state).

Output:
- New blockchain state `state′`.

Procedure:
1. Let `state′` be `state`.
2. For each [nonce entry](zkvm-spec.md#nonce-entry) `n` in `txlog`:
   1. Verify `n.blockid` is one of the following:
      - The [ID](#block-id) of `state.initialheader`,
        or
      - One of the block ids in `state.refids`.
   2. Verify `n.nonce_anchor` is _not_ equal to any anchor in `state.nonces`.
   3. Add the pair `<n.nonce_anchor,n.maxtime_ms>` to `state′.nonces`.
3. For each [input entry](zkvm-spec.md#input-entry) or [output entry](zkvm-spec.md#output-entry) in `txlog`:
   1. If an input entry,
      verify its ID is in `state′.utxos`,
      then remove it.
   2. If an output entry,
      add its utxo ID to `state′.utxos`.
4. Return `state′`.

Note: utxos may be consumed in the same block,
or even the same transaction,
in which they are created.
Implementations should therefore not try to reorder or batch step 3,
at least not without taking extra care to cancel out such “local pairs” first.

## Compute txroot

Input:
- Ordered list `txids` of [transaction IDs](zkvm-spec.md#transaction-id).

Output:
- [Merkle root hash](zkvm-spec.md#merkle-binary-tree) of the transaction list.

Procedure:
1. Create a [transcript](zkvm-spec.md#transcript) `T` with label `transaction_ids`.
2. Return `MerkleHash(T, txids)`.

## Compute utxoroot

Input:
- Unordered set `utxos` of [utxo IDs](zkvm-spec.md#utxo).

Output:
- [Merkle patricia root hash](#merkle-patricia-tree) of the given utxos.

Procedure:
1. Create a [transcript](zkvm-spec.md#transcript) `T` with label `utxos`.
2. Return `MPTH(T, utxos)`.

## Compute nonceroot

Input:
- Unordered set `nonces` of [nonce anchors](zkvm-spec.md#nonce).

Output:
- [Merkle patricia root hash](#merkle-patricia-tree) of the given nonce anchors.

Procedure:
1. Create a [transcript](zkvm-spec.md#transcript) `T` with label `nonces`.
2. Return `MPTH(T, nonces)`.
