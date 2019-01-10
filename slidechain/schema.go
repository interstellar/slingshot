package main

const schema = `
CREATE TABLE IF NOT EXISTS blocks (
  height INTEGER NOT NULL PRIMARY KEY,
  hash BLOB NOT NULL UNIQUE,
  bits BLOB NOT NULL
);

CREATE TABLE IF NOT EXISTS snapshots (
  height INTEGER NOT NULL PRIMARY KEY,
  bits BLOB NOT NULL
);

CREATE TABLE IF NOT EXISTS pegs (
  txid TEXT NOT NULL,
  txhash BLOB NOT NULL,
  operation_num INTEGER NOT NULL,
  amount INTEGER NOT NULL,
  asset_xdr BLOB NOT NULL,
  imported INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS exports (
  txid TEXT NOT NULL,
  recipient TEXT NOT NULL,
  amount INTEGER NOT NULL,
  asset_xdr BLOB NOT NULL,
  exported INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS custodian_account (
  account_id TEXT NOT NULL PRIMARY KEY
);
`
