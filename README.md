# Project Slingshot

_Accelerating trajectory into the interstellar space._

Slingshot is a new blockchain architecture under heavy development,
with strong focus on scalability, privacy and safety.

The Slingshot project consists of the following sub-protocols and components:

### [Slidechain](slidechain)

This is Slidechain, a demonstration of a minimal [Stellar](https://stellar.org/) sidechain.
Slidechain allows you to _peg_ funds from the Stellar testnet, _import_ then to a _sidechain_,
and later _export_ them back to Stellar.

* [Slidechain README](slidechain/Readme.md)
* [Pegging specification](slidechain/Pegging.md)
* [Demo documentation](slidechain/Running.md)

### [ZkVM](zkvm)

An evolution of [TxVM](https://github.com/chain/txvm) with **cloaked assets** and **zero-knowledge smart contracts**.

* [README](zkvm/README.md)
* [Specification](zkvm/docs/zkvm-spec.md)
* [API guide](zkvm/docs/zkvm-api.md)

### [Spacesuit](spacesuit)

Interstellar’s implementation of _Cloak_, a confidential assets protocol
based on the Bulletproofs zero-knowledge circuit proof system.

* [Spacesuit README](spacesuit/README.md)
* [Cloak specification](spacesuit/spec.md)

### [Keytree](keytree)

A _key blinding scheme_ for deriving hierarchies of public keys for [Ristretto](https://ristretto.group)-based signatures.

* [Specification](keytree/keytree.md)

