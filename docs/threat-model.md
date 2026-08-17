# What actually secures this

The most common misreading of this design is that the fingerprint's uniqueness is the
security control. It is not. Being precise about this determines where the real risks are.

**The desktop never sees a fingerprint.** It never receives one, never stores one, never
compares one. No Android API would give it one even if we wanted it. The fingerprint match
happens entirely inside the phone's TEE, and its only output is permission to use one
private key for one operation.

What the desktop verifies is an ECDSA signature from **one specific public key** recorded at
pairing time. The chain is:

1. The desktop trusts exactly one auth public key per paired device.
2. Only that phone's TEE holds the matching private key. It is hardware-backed and
   non-extractable, and key attestation proved that at pairing.
3. The TEE will only use that key after a `BIOMETRIC_STRONG` match against the fingerprints
   enrolled **on that phone**, per use (`authTimeout` absent, verified at pairing).

So the key is the security. The fingerprint is the local gate on the key.

## Will another device work?

**No — categorically.** An attacker's phone holds a different key, so its signature fails
verification. It does not matter whose finger touches it, how good the sensor is, or whether
the attacker also runs our app. Pairing is what binds a device, and it costs the desktop
password (`pair-device` is polkit `auth_self`) plus an out-of-band QR scan and a six-digit
SAS comparison.

Several phones can be paired, each with its own key, each revocable individually from the
KCM.

## What the BLE nonce is, and is not

It is a **location proof, not a capability**. There is no BLE connection, no pairing, no
GATT — the desktop broadcasts and the phone listens passively. An attacker in radio range
can therefore *read* the nonce trivially. That is fine: knowing the nonce grants nothing,
because producing a valid assertion still requires the phone's non-extractable key. The
nonce's only job is to be **un-hearable from far away**, which is exactly the property that
excludes a network-only attacker.

Consequences:

- Nonce disclosure to someone in the room is harmless on its own.
- Replay is dead anyway: single-use nonce, per-verification challenge, TLS exporter binding,
  monotonic counter.
- A **relay** needs equipment near the desktop *and* near the phone simultaneously. Even
  then it only defeats the proximity gate — the phone still shows the sheet naming host,
  user and action, and still demands an Approve tap and a live fingerprint. A relay alone
  unlocks nothing.
- Minor leak, accepted: anyone in range learns that a verification is in progress and that
  this machine runs kpasskey (the service UUID is a static identifier).

## Where the biometric risk actually is

Not in uniqueness — in enrolment and in spoofing, and both require the physical phone.

- **An attacker enrols their own fingerprint on your phone.** This is the sharpest risk,
  and it needs your phone's PIN or passcode. Mitigated by
  `setInvalidatedByBiometricEnrollment(true)`: enrolling a new print destroys the key,
  producing `KeyPermanentlyInvalidatedException`, after which the desktop demands re-pairing
  — which costs the desktop password. **OEM behaviour here has historically been
  inconsistent, so this must be verified empirically on the real device in Phase 4.**
- **Someone already enrolled on your phone** (a partner who added a finger legitimately) can
  unlock the desktop. Nothing in this design detects that; it is a property of the phone.
- **Spoofing.** Fingerprints are not secrets — you leave them on every glass you touch.
  Android's Class 3 (`BIOMETRIC_STRONG`) bar tolerates a nonzero spoof-acceptance rate (the
  CDD threshold is on the order of 7%), so a determined mould attack is in scope for a
  motivated attacker. Uniqueness is not unforgeability. The attacker still needs your
  physical phone in hand, because the key never leaves it.

## The full chain an attacker must complete

To unlock the desktop through this path, all of these at once:

1. **Your physical phone** — irreplaceable, the key is in its TEE.
2. **Satisfy its biometric** — your finger, a good spoof, or an enrolment they performed
   using your phone PIN (which should have invalidated the key).
3. **Radio proximity to the desktop** — to hear a −34 dBm broadcast.
4. **Network reachability to the desktop** — to carry the TLS session.
5. **Tap Approve** on a sheet that names the host, the user and the action.

## The honest ceiling

The password fallback stays enabled, so an attacker picks whichever path is weaker. **The
account is as strong as the weaker of the password and this chain**, not the stronger. That
is inherent to a replacement factor rather than a second factor, and it was a deliberate
choice — see the decision table in the plan.

Anyone with root on the desktop bypasses all of this; that machine is already lost. The
containment property that still holds is that assertions are bound to `hostId`, so a
compromised desktop cannot mint one that a different desktop will accept.
