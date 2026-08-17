package org.kpasskey.net

import org.json.JSONObject
import java.util.Base64

/**
 * A verification request as it arrived.
 *
 * [raw] is load-bearing. The desktop verifies the signature against the exact bytes it sent,
 * so signing must cover this string — never a re-serialisation of the parsed fields, which
 * would differ in key order or spacing and fail verification in a way that looks like a
 * broken key rather than a broken encoder.
 */
data class VerifyRequest(
    val raw: String,
    val id: Long,
    val user: String,
    val host: String,
    val action: String,
    val detail: String,
) {
    /** What the approval screen leads with, e.g. "Unlock lenovo as iharob". */
    fun sentence(): String =
        when (action) {
            "unlock" -> "Unlock $host as $user"
            "login" -> "Log in to $host as $user"
            "sudo" -> "Run a command as $user on $host"
            "polkit" -> "Authorise an administrator action on $host"
            "webauthn" -> "Sign in to a website from $host"
            "test" -> "Test request from $host"
            else -> "$action on $host as $user"
        }
}

sealed interface DesktopMessage {
    data class Verify(val request: VerifyRequest) : DesktopMessage

    data class PairResult(val ok: Boolean, val deviceId: String, val reason: String) : DesktopMessage

    data class WebAuthnCreate(val id: Long, val rpId: String, val userName: String) : DesktopMessage

    /** [payload] is `authData ‖ clientDataHash`, base64, to be signed verbatim. */
    data class WebAuthnAssert(
        val id: Long,
        val rpId: String,
        val credentialId: String,
        val payload: String,
    ) : DesktopMessage
}

/** Something waiting on the user. The approval screen renders whichever arrives. */
sealed interface Approval {
    val id: Long

    fun sentence(): String

    fun detail(): String

    data class Sign(val request: VerifyRequest) : Approval {
        override val id: Long get() = request.id

        override fun sentence(): String = request.sentence()

        override fun detail(): String = request.detail
    }

    data class CreateCredential(val message: DesktopMessage.WebAuthnCreate) : Approval {
        override val id: Long get() = message.id

        override fun sentence(): String = "Create a passkey for ${message.rpId}"

        override fun detail(): String = message.userName
    }

    data class AssertCredential(val message: DesktopMessage.WebAuthnAssert) : Approval {
        override val id: Long get() = message.id

        override fun sentence(): String = "Sign in to ${message.rpId}"

        override fun detail(): String = ""
    }
}

/**
 * Returns null for anything unrecognised rather than throwing: the link must survive a
 * message from a newer desktop without dropping the connection.
 *
 * Note the asymmetry in the wire format — a verification is tagged `type`, pairing is
 * tagged `t`.
 */
fun parseDesktopMessage(line: String): DesktopMessage? {
    val json = runCatching { JSONObject(line) }.getOrNull() ?: return null

    if (json.optString("t") == "pair.result") {
        return DesktopMessage.PairResult(
            ok = json.optBoolean("ok", false),
            deviceId = json.optString("id"),
            reason = json.optString("reason"),
        )
    }

    when (json.optString("type")) {
        "webauthn.makecred" ->
            return DesktopMessage.WebAuthnCreate(
                id = json.optLong("id", -1),
                rpId = json.optString("rpId"),
                userName = json.optString("userName"),
            )
        "webauthn.assert" ->
            return DesktopMessage.WebAuthnAssert(
                id = json.optLong("id", -1),
                rpId = json.optString("rpId"),
                credentialId = json.optString("credId"),
                payload = json.optString("payload"),
            )
        "verify.request" -> Unit
        else -> return null
    }
    val id = json.optLong("id", -1)
    if (id < 0) {
        return null
    }
    return DesktopMessage.Verify(
        VerifyRequest(
            raw = line,
            id = id,
            user = json.optString("user"),
            host = json.optString("host"),
            action = json.optString("action"),
            detail = json.optString("detail"),
        ),
    )
}

/** Standard base64 with padding — the pairing token is url-safe, this deliberately is not. */
fun approvalLine(id: Long, signature: ByteArray): String =
    JSONObject()
        .put("id", id)
        .put("sig", Base64.getEncoder().encodeToString(signature))
        .toString()

fun denialLine(id: Long, reason: String): String =
    JSONObject().put("id", id).put("denied", reason).toString()

fun credentialLine(id: Long, credentialId: String, publicKeyPem: String): String =
    JSONObject()
        .put("id", id)
        .put("credId", credentialId)
        .put("publicKeyPem", publicKeyPem)
        .toString()

fun pairRequestLine(
    token: String,
    publicKeyPem: String,
    name: String,
    model: String,
    securityLevel: String,
    verifiedBoot: String,
): String =
    JSONObject()
        .put("t", "pair.request")
        .put("token", token)
        .put("publicKeyPem", publicKeyPem)
        .put("name", name)
        .put("model", model)
        .put("securityLevel", securityLevel)
        .put("verifiedBoot", verifiedBoot)
        .toString()
