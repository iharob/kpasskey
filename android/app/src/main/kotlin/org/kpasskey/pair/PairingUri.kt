package org.kpasskey.pair

import android.net.Uri
import java.util.Base64

data class PairingInvite(
    val hostName: String,
    val token: String,
    val address: String,
    val port: Int,
    /** Recomputed locally — the desktop never puts this on the wire. */
    val confirmationCode: String,
)

/**
 * Parses `kpk://pair?v=1&h=<host>&t=<base64url-no-pad token>&a=<addr>:<port>`.
 * Returns null for anything else, so pointing the scanner at an unrelated QR is a no-op
 * rather than an error to dismiss.
 */
fun parsePairingUri(text: String): PairingInvite? {
    val uri = runCatching { Uri.parse(text) }.getOrNull() ?: return null
    if (uri.scheme != "kpk" || uri.host != "pair") {
        return null
    }

    val token = uri.getQueryParameter("t").orEmpty()
    val decoded = runCatching { Base64.getUrlDecoder().decode(token) }.getOrNull() ?: return null
    if (decoded.size < TOKEN_PREFIX) {
        return null
    }

    val endpoint = uri.getQueryParameter("a").orEmpty()
    val separator = endpoint.lastIndexOf(':')
    if (separator <= 0) {
        return null
    }
    val port = endpoint.substring(separator + 1).toIntOrNull() ?: return null

    return PairingInvite(
        hostName = uri.getQueryParameter("h").orEmpty().ifEmpty { "desktop" },
        token = token,
        address = endpoint.substring(0, separator),
        port = port,
        confirmationCode = confirmationCode(decoded),
    )
}

/**
 * Mirrors the desktop's `short_code` in `daemon/crates/kpk-spike/src/pairing.rs`: fold the
 * first four token bytes with `value * 31 + byte`, modulo a million. `UInt` is required —
 * the desktop wraps as `u32`, and Kotlin's signed `Int` would give a negative remainder.
 */
private fun confirmationCode(token: ByteArray): String {
    val value = token.take(TOKEN_PREFIX).fold(0u) { accumulator, byte ->
        accumulator * 31u + (byte.toUInt() and 0xFFu)
    }
    return (value % 1_000_000u).toString().padStart(6, '0')
}

private const val TOKEN_PREFIX = 4
