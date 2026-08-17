# PAM ground truth

Measured with `pam/tests/kpk_pamprobe.c`, which calls `pam_start`/`pam_authenticate` with a
conversation function that refuses to answer prompts — the same posture kscreenlocker's
non-interactive authenticators have.

Measured on: pam 1.7.2-2, kscreenlocker 6.7.4-1, kernel 7.1.8-arch1-3, **fprintd absent**,
as the unprivileged user.

| Service | `pam_authenticate` | Meaning to kscreenlocker |
|---|---|---|
| `kde-fingerprint` | 28 `PAM_MODULE_UNKNOWN` | authenticator unavailable, UI hides it |
| `kde-smartcard` | 28 `PAM_MODULE_UNKNOWN` | authenticator unavailable, UI hides it |
| `kde` | 20 `PAM_AUTHTOK_ERR` (prompt refused) | interactive stack; prompts for a password |
| service with no pam.d file | 7 `PAM_AUTH_ERR` (via `other` → `pam_deny`) | generic failure |

## Why `kde-fingerprint` returns 28 and not 0

The vendor stack at `/usr/lib/pam.d/kde-fingerprint` contains
`-auth required pam_fprintd.so`, and `pam_fprintd.so` is not installed here.

`man 5 pam.conf`: the `-` prefix means only that "the PAM library will not log to the system
log if it is not possible to load the module". It does **not** skip the module. The module
still fails to load and yields `PAM_MODULE_UNKNOWN`, which the `required` control propagates
as the stack result.

This matters because the rest of that stack (`pam_shells`, `pam_nologin`,
`pam_faillock preauth`, `pam_permit`, `pam_env`) would otherwise all succeed and the stack
would grant access with no authentication whatsoever. **It does not** — measured, not
assumed. `PAM_MODULE_UNKNOWN` is also one of the two codes
`PamWorker::authenticate()` treats as "unavailable", which is why an Arch system without
fprintd shows no fingerprint affordance rather than a broken one.

## Consequences for this project

- Our module goes in the `pam_fprintd` slot and **must not carry a `-` prefix**: if
  `pam_kpasskey.so` is ever missing, `PAM_MODULE_UNKNOWN` must reach `pam_authenticate`
  so the affordance disappears. With `[success=done default=die]` it does.
- Both failure-to-load paths are fail-closed, but they differ: a missing **module** gives 28
  (affordance hidden), a missing **service file** gives 7 (generic failure toast). Prefer the
  former; never ship a stack that can reach `pam_permit` as its last word.
- The interactive `kde` stack is untouched by any of our changes, so the password path stays
  available even if everything about this project is broken.

## Reproducing

```
gcc -Wall -Wextra -Werror -Wconversion -D_FORTIFY_SOURCE=3 -O2 \
    -o kpk_pamprobe pam/tests/kpk_pamprobe.c -lpam
./kpk_pamprobe kde-fingerprint
```

No root required, and it modifies nothing.
