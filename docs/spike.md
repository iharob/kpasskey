# Walking skeleton — first end-to-end result

**2026-08-16.** A real `pkexec id` on this desktop was authorised by a fingerprint on a
Motorola moto g55 5G, with no password typed:

```
$ pkexec id
uid=0(root) gid=0(root) grupos=0(root),992(kvm)
pkexec exit=0

[pam] uid=0 user=root action=polkit-1 detail=authorising polkit-1
[phone] sent request 4, awaiting approval
[pam] -> APPROVED (signature verified)
```

## What is real

- The signing key is EC P-256 in the phone's **TEE**, non-extractable, created with
  `setUserAuthenticationRequired(true)` and auth-per-use, and **hardware-attested** — see
  `docs/attestation-findings.md`.
- Each signature requires a **fresh fingerprint touch**; `BiometricPrompt` is bound to the
  operation through `CryptoObject`, with no PIN fallback offered.
- The desktop verifies an **ECDSA signature** against the public key extracted from the
  attestation chain. `kpk-spike self-test` proves the three properties this rests on: a
  correct signature verifies, a signature over altered bytes fails (the `user` field flipped
  to `root`), and a signature from an unenrolled key is rejected.
- The signed payload is the **entire request** — user, host, action, detail — not a bare
  nonce, so an approval cannot be lifted onto a different action.
- `pkexec` really did run as uid 0 through the normal polkit path.

## What is a shortcut

Every one of these is replaced by the real daemon:

| Spike | Real design |
|---|---|
| Plain TCP over `adb reverse` | TLS 1.3, mutual SPKI pinning, mDNS discovery |
| Newline-delimited JSON | CBOR, signing bytes exactly as transmitted |
| Unix socket, daemon runs as the desktop user | System D-Bus, daemon as a `kpasskey` system user |
| No proximity check | Mandatory BLE proximity nonce, fail-closed |
| Public key extracted by hand with `openssl` | Pairing flow with QR, SAS, and attestation verified at run time |
| One hard-coded device | Device store, revocation, per-user policy |
| App must be foregrounded | Foreground service + full-screen-intent approval activity |

**The trust relationship is inverted in the spike**: the daemon runs as the desktop user
while the PAM module runs as root under polkit, so a root authorisation depends on a
user-owned socket. Fine on one machine for a bench demo; never shippable.

## Two findings worth keeping

**polkit's sandbox dictates where the daemon can listen.** `polkit.service` sets
`PrivateTmp=yes`, `ProtectHome=yes`, `ProtectSystem=strict` and `PrivateNetwork=yes`, and
`polkit-agent-helper-1` is no longer setuid — polkitd spawns it, so the `polkit-1` PAM stack
runs inside those namespaces. A socket under `/tmp`, `/var/tmp` or `/run/user/<uid>` is
invisible to it, and **no TCP transport can work at all**. `/run/kpasskey/` is reachable:
`PrivateTmp` does not cover `/run`, and although `ProtectSystem=strict` mounts it read-only,
`sb_permission()` only returns `EROFS` for regular files, directories and symlinks —
`S_IFSOCK` is exempt. The system bus socket is reachable for the same reason, so the real
design's transport is validated rather than changed.

**Replies must be routed by request id, not by socket.** Twice a verification was approved
— the phone produced a valid signature — and the assertion was then lost because the link
had dropped underneath it. The daemon now keeps a pending map keyed by id, so a reply
arriving on a *new* connection still finds its waiter, and the client retries delivery across
reconnects. This also removed the need to tear down the link on timeout: a late reply simply
finds no waiter and is discarded.

## Reproducing

```
# desktop
cargo build --release -p kpk-spike
sudo install -d -o "$USER" -g "$USER" -m 0755 /run/kpasskey
./target/release/kpk-spike serve \
    --pubkey crates/kpk-attest/testdata/device-pubkey.pem \
    --socket /run/kpasskey/pam.sock --port 34719 --timeout 180

# phone (app must be installed, key generated, and foregrounded)
adb reverse tcp:34719 tcp:34719
adb shell am start -n org.kpasskey/.ui.MainActivity

# PAM
sudo install -m0755 pam_kpk_spike.so /usr/lib/security/pam_kpk_spike.so
sudo install -m0644 pam/tests/polkit-1.spike /etc/pam.d/polkit-1
pkexec id
```

Revert with `sudo rm -f /etc/pam.d/polkit-1 /usr/lib/security/pam_kpk_spike.so`.

Do not run other `pkexec` instances or kill stray ones while a prompt is pending — cancelling
one polkit dialog cancels the authentication that a pending phone approval belongs to.
