# 🔗 Secret Sharing Guide

CryptEnv offers three secure methods for sharing secrets with team members without third-party plain-text services:

1. **LAN Bridge** — Real-time local network sharing via mDNS and ECDH.
2. **Encrypted Packages** — Offline portable `.vault` files with Argon2id-derived keys.
3. **Internet Relay** — Remote team sharing via an ephemeral, burn-after-read Supabase table.

---

## 1. LAN Bridge (Local Network, Real-Time)

Ideal for colleagues on the same Wi-Fi / local network.

### How It Works:
- **Discovery**: Uses encrypted mDNS pairing.
- **Key Exchange**: X25519 ECDH for forward secrecy.
- **Pairing & Verification**: Sender displays a 6-digit code (valid 5 min) and both parties verify a SHA-256 fingerprint of the exchanged keys.
- **Data Transfer**: Direct AES-256-GCM encrypted TCP stream.

---

## 2. Encrypted Package (Offline / Portable)

Ideal for asynchronous sharing via email, Slack, or USB drives.

### How It Works:
- Sender exports items into a self-contained `.vault` package:
  ```bash
  crypt-env share export api_key_1 api_key_2 -o secrets.vault
  ```
- Generates a random 12-character high-entropy passphrase.
- Encrypted using **AES-256-GCM** with keys derived via **Argon2id** (32MB memory cost).
- Receiver imports the file by entering the passphrase out-of-band:
  ```bash
  crypt-env share import -f secrets.vault
  ```

---

## 3. Internet Relay (Remote Teams)

For teams working remotely across different networks, CryptEnv provides ephemeral cloud relay sharing through Supabase.

### Security Highlights:
- **Zero-Knowledge Relay**: Supabase stores only ciphertext, salt, and nonce. The decryption passphrase **never touches the server**.
- **Burn-After-Read**: Payload is automatically deleted upon first retrieval.
- **24-Hour TTL**: Unretrieved payloads expire and are purged automatically.

### Supabase Relay Setup (One-Time)

1. Create a free project at [supabase.com](https://supabase.com).
2. Run this SQL in the **SQL Editor**:

```sql
CREATE TABLE relay (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    code_hash TEXT UNIQUE NOT NULL,
    encrypted_payload BYTEA NOT NULL,
    salt BYTEA NOT NULL,
    nonce BYTEA NOT NULL,
    created_at TIMESTAMP DEFAULT now(),
    expires_at TIMESTAMP DEFAULT now() + INTERVAL '24 hours',
    accessed BOOLEAN DEFAULT FALSE
);

CREATE INDEX idx_relay_code_hash ON relay(code_hash);
CREATE INDEX idx_relay_expires_at ON relay(expires_at);
```

3. In CryptEnv desktop app (**Settings** → **Internet Sharing**), input:
   - Supabase Project URL
   - Supabase `anon` public key
4. Click **Test Connectivity**.

---

## 📋 Audit Trail

All sharing operations (sending, receiving, exporting, importing) are recorded in the local SQLite `share_log` table with timestamps and fingerprint hashes for local auditing.
