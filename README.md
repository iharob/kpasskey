# kpasskey

Use an Android phone as a passkey for a KDE desktop: when the desktop asks for a
password, you approve it on the phone with a fingerprint instead. The desktop
verifies an ECDSA signature from a hardware-attested key held in the phone's TEE.

## Status

This is a **walking skeleton**, not a shipping product. One real `pkexec id` has been
authorised end to end with no password typed, and the cryptographic core is real: a
non-extractable P-256 key in the phone's TEE, auth-per-use, hardware-attested, signing
the entire request rather than a bare nonce.

Everything around that core is still a bench shortcut — plain TCP over `adb reverse`,
newline-delimited JSON, one hard-coded device, no proximity check, and a daemon running
as the desktop user while the PAM module runs as root. `docs/spike.md` lists what is real
and what is a shortcut, side by side. Do not put this on a machine you care about.

## How it works

The desktop never sees a fingerprint and could not obtain one if it wanted to. The match
happens inside the phone's TEE, and its only output is permission to use one private key
for one operation. What the desktop checks is a signature from **one specific public key**
recorded at pairing time. `docs/threat-model.md` is the argument for why that is the
security control, and where the real ceiling is.

## Components

| Part | Name | Notes |
|---|---|---|
| Daemon | `kpk-spike` (Rust) | Unix socket `/run/kpasskey/pam.sock`, D-Bus `org.kpasskey` |
| PAM module | `pam_kpk_spike.so` | Blocks on the socket until the phone answers |
| Plasma KCM | `kcm_kpasskey` | System Settings → Phone Passkey |
| Android app | `org.kpasskey` | Foreground service + approval activity |

Paired devices live in `~/.local/share/kpasskey/devices`. Pairing is carried over a
`kpk://pair?v=1&…` URI.

## Building

Needs Rust 1.90+, Qt 6.8+, KF6 6.0+ with ECM, a C compiler and `libpam`. All three
languages build with warnings as errors — `-Werror`, `warnings = "deny"`,
`allWarningsAsErrors` — so a warning fails the build.

```sh
make                      # daemon, PAM module, KCM
sudo make install         # PREFIX=/usr by default; DESTDIR= for packaging
```

The systemd units ship pointing at the build tree so a rebuild takes effect on restart;
`make install` rewrites `ExecStart` to the installed binary.

The Android half is a separate Gradle build (JDK 17, compileSdk 36, minSdk 31):

```sh
cd android && ./gradlew assembleDebug
```

### PAM is never installed for you

`make install` does not write to `/etc/pam.d`. A reference stack is installed to
`$PREFIX/share/kpasskey/polkit-1.example`; putting it in place is a deliberate manual
step, because the failure mode of a bad auth stack is being unable to log in. Do it with
a root shell already open on another TTY.

Removing an override under `/etc/pam.d` restores the distro default rather than breaking
the service, because PAM falls back to `/usr/lib/pam.d/<service>` — but confirm that
vendor file exists first. Without it the service falls through to `/etc/pam.d/other`,
which denies everything.

## Documentation

| File | What it covers |
|---|---|
| `docs/spike.md` | What the end-to-end result proves, and every shortcut behind it |
| `docs/threat-model.md` | What actually secures this, and the honest ceiling |
| `docs/pam-behaviour.md` | Measured PAM return codes and what kscreenlocker does with them |
| `docs/attestation-findings.md` | The attestation chain as captured from real hardware |

## License

Copyright (C) 2026 Iharob Al Asimi.

kpasskey is free software: you can redistribute it and/or modify it under the terms of
the GNU General Public License as published by the Free Software Foundation, **either
version 2 of the License, or (at your option) any later version**.

This program is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY;
without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
See the GNU General Public License for more details.

The full text of version 2 is in [LICENSE](LICENSE). SPDX identifier:
`GPL-2.0-or-later`.
