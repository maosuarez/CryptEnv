#!/usr/bin/env node
// Verifies Tauri updater signatures against the minisign pubkey embedded in
// src-tauri/tauri.conf.json — the same key installed apps use to accept or
// reject an update. Catches a TAURI_SIGNING_PRIVATE_KEY that doesn't match the
// pubkey, which the build itself never notices (it signs happily with any key).
//
// Usage: node scripts/verify-updater-sig.mjs <file> [<file> ...]
//        Each <file> must have a sibling <file>.sig (as produced by `tauri build`
//        or `tauri signer sign`).
// Env:   TAURI_CONF — alternate tauri.conf.json path (default: src-tauri/tauri.conf.json)
//
// Format (minisign, each Tauri value is base64 of the minisign text file):
//   pubkey line 2:   b64( "Ed" | keyid[8] | ed25519_pk[32] )
//   sig    line 2:   b64( alg[2] | keyid[8] | sig[64] )  alg "ED" = prehashed (BLAKE2b-512)
//   sig    line 3:   "trusted comment: ..."
//   sig    line 4:   b64( ed25519 sig over sig[64] || trusted_comment )

import { readFileSync } from 'node:fs';
import { createHash, createPublicKey, verify } from 'node:crypto';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const confPath = process.env.TAURI_CONF || join(root, 'src-tauri', 'tauri.conf.json');

function fail(msg) {
  console.error(`error: ${msg}`);
  process.exit(1);
}

/** Decodes a Tauri base64 wrapper into the minisign text lines. */
function minisignLines(b64, what) {
  const text = Buffer.from(b64.trim(), 'base64').toString('utf8');
  const lines = text.split(/\r?\n/).filter((l) => l.length > 0);
  if (lines.length < 2 || !lines[0].startsWith('untrusted comment:')) {
    fail(`${what} is not a base64-encoded minisign file`);
  }
  return lines;
}

function loadPubkey() {
  const conf = JSON.parse(readFileSync(confPath, 'utf8'));
  const b64 = conf?.plugins?.updater?.pubkey;
  if (!b64) fail(`no plugins.updater.pubkey in ${confPath}`);
  const raw = Buffer.from(minisignLines(b64, 'pubkey')[1], 'base64');
  if (raw.length !== 42 || raw.subarray(0, 2).toString() !== 'Ed') fail('unsupported pubkey format');
  const keyId = raw.subarray(2, 10);
  const key = createPublicKey({
    key: { kty: 'OKP', crv: 'Ed25519', x: raw.subarray(10).toString('base64url') },
    format: 'jwk',
  });
  return { keyId, key };
}

function hexId(buf) {
  return Buffer.from(buf).reverse().toString('hex').toUpperCase();
}

function verifyFile(file, pub) {
  let sigB64;
  try {
    sigB64 = readFileSync(`${file}.sig`, 'utf8');
  } catch {
    return `${file}.sig not found`;
  }
  if (!sigB64.trim()) return `${file}.sig is empty`;
  const lines = minisignLines(sigB64, `${file}.sig`);
  if (lines.length < 4 || !lines[2].startsWith('trusted comment: ')) return `${file}.sig is malformed`;

  const raw = Buffer.from(lines[1], 'base64');
  if (raw.length !== 74) return `${file}.sig has an unexpected length`;
  const alg = raw.subarray(0, 2).toString();
  const keyId = raw.subarray(2, 10);
  const sig = raw.subarray(10);

  if (!keyId.equals(pub.keyId)) {
    return `${file}: signed with key ${hexId(keyId)}, but tauri.conf.json trusts ${hexId(pub.keyId)}`;
  }

  const data = readFileSync(file);
  let message;
  if (alg === 'ED') message = createHash('blake2b512').update(data).digest();
  else if (alg === 'Ed') message = data;
  else return `${file}.sig uses unknown algorithm "${alg}"`;

  if (!verify(null, message, pub.key, sig)) return `${file}: signature does not match file contents`;

  const trusted = Buffer.from(lines[2].slice('trusted comment: '.length), 'utf8');
  const globalSig = Buffer.from(lines[3], 'base64');
  if (!verify(null, Buffer.concat([sig, trusted]), pub.key, globalSig)) {
    return `${file}: trusted comment signature is invalid`;
  }
  return null;
}

const files = process.argv.slice(2);
if (files.length === 0) fail('usage: verify-updater-sig.mjs <file> [<file> ...]');

const pub = loadPubkey();
console.log(`trusted key id: ${hexId(pub.keyId)}`);
let failed = 0;
for (const file of files) {
  const problem = verifyFile(file, pub);
  if (problem) {
    console.error(`FAIL ${problem}`);
    failed++;
  } else {
    console.log(`OK   ${file}`);
  }
}
process.exit(failed ? 1 : 0);
