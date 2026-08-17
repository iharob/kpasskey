# Attestation on real hardware — Motorola moto g55 5G

Captured 2026-08-16 from the bench app. Fixtures live in
`daemon/crates/kpk-attest/testdata/` and are the reference the Rust verifier is built
against, so this is measurement rather than specification.

## Device

| | |
|---|---|
| Model | motorola moto g55 5G (`taipei`) |
| Android | 16 (SDK 36), security patch 2026-07-01 |
| Keystore | `hardware_keystore=300` (KeyMint 3.0), `keystore.app_attest_key` |
| StrongBox | **absent** — no `android.hardware.strongbox_keystore` feature |
| Verified boot | `green`, bootloader locked |

The StrongBox fallback in `AuthKeyStore.generate` is therefore load-bearing on this device,
not a theoretical branch.

## Chain

Five certificates, and the root matters:

```
leaf  CN=Android Keystore Key
  ->  O=TEE, CN=<redacted: 32 hex chars, a per-device attestation key id>
  ->  O=Google LLC, CN=Droid CA3
  ->  O=Google LLC, CN=Droid CA2
  ->  CN=Key Attestation CA1, OU=Android, O=Google LLC, C=US   (self-signed, P-384)
```

`openssl verify -CAfile <root> -untrusted <intermediates> leaf.pem` → **OK**.

**This is the new post-April-2026 RKP root, not the legacy one.** A verifier carrying only
the legacy Google root would reject this phone outright. The plan's requirement to trust
both roots is confirmed by hardware, not inferred from release notes.

## Decoded KeyDescription (OID 1.3.6.1.4.1.11129.2.1.17)

| Field | Value | Verdict |
|---|---|---|
| `attestationVersion` | 300 | |
| `attestationSecurityLevel` | 1 = **TrustedEnvironment** | accepted |
| `keyMintVersion` / level | 300 / 1 = TrustedEnvironment | accepted |
| `attestationChallenge` | 32 bytes, matches the app's nonce | binds key to this pairing |
| `uniqueId` | empty | |

`softwareEnforced`:

| Tag | Field | Value |
|---|---|---|
| 701 | `creationDateTime` | present |
| 709 | `attestationApplicationId` | package `org.kpasskey`, signing-cert SHA-256 `FE058B7A…C61C` |

`hardwareEnforced` — every check the design demands:

| Tag | Field | Value | Required | Pass |
|---|---|---|---|---|
| 1 | `purpose` | {2} = SIGN | ⊇ SIGN | ✅ |
| 2 | `algorithm` | 3 = EC | EC | ✅ |
| 3 | `keySize` | 256 | 256 | ✅ |
| 5 | `digest` | {4} = SHA-2-256 | ⊇ SHA-256 | ✅ |
| 10 | `ecCurve` | 1 = P-256 | P-256 | ✅ |
| 503 | `noAuthRequired` | **absent** | absent | ✅ |
| 504 | `userAuthType` | **2 = FINGERPRINT only** | fingerprint set, `PASSWORD` (1) clear | ✅ |
| 505 | `authTimeout` | **absent** | absent ⇒ auth-per-use | ✅ |
| 509 | `unlockedDeviceRequired` | NULL, present | present | ✅ |
| 702 | `origin` | 0 = GENERATED | GENERATED | ✅ |
| 704 | `rootOfTrust` | `deviceLocked = TRUE`, `verifiedBootState = 0 (Verified)` | both | ✅ |
| 705 | `osVersion` | 160000 (Android 16.0.0) | — | |
| 706 | `osPatchLevel` | 202607 | — | |
| 718/719 | vendor/boot patch | 20260701 | — | |

Tags **504 and 505 together are the whole point**: `userAuthType` proves only a fingerprint
can release the key (a PIN cannot), and the absence of `authTimeout` proves each signature
needs its own fresh touch rather than opening a time window. Both hold here.

## The gap: invalidation is not attestable

`setInvalidatedByBiometricEnrollment(true)` has **no KeyMint tag** and therefore appears
nowhere in the attestation. The desktop cannot verify it cryptographically at pairing; it is
enforced locally by keystore binding the key to the current biometric SID, so enrolling a new
fingerprint changes the SID and orphans the key.

Consequences:

- The plan's threat-model claim that re-pairing is forced after a hostile enrolment rests on
  a property the desktop **cannot check**. It must be established empirically per device
  model, and re-tested after major OS updates.
- `KeyInfo.isInvalidatedByBiometricEnrollment` reports the local flag, which is worth
  surfacing during pairing as advisory information — but it is self-reported by the phone and
  is not evidence to a verifier.
- **Still outstanding on this device**: enrol an additional fingerprint and confirm signing
  then throws `KeyPermanentlyInvalidatedException`.

## Signing

`Signed 32 bytes -> 71-byte signature; verifies locally: true`, with the fingerprint prompt
offering no PIN fallback. The system log independently corroborates the hardware path:

```
FingerprintAuthenticationClient ... owner=org.kpasskey ... success: true
keystore2: add_auth_token(challenge=…, authType=0x2, …)
```

`authType=0x2` is `HW_AUTH_FINGERPRINT`, and the auth token carries an operation challenge —
the per-use binding, observed rather than assumed.
