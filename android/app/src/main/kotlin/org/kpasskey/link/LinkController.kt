package org.kpasskey.link

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import org.kpasskey.crypto.AuthKeyStore
import org.kpasskey.net.Approval
import org.kpasskey.net.DesktopLink
import org.kpasskey.net.DesktopMessage
import org.kpasskey.net.LinkState
import org.kpasskey.net.VerifyRequest
import org.kpasskey.net.approvalLine
import org.kpasskey.net.credentialLine
import org.kpasskey.net.denialLine
import org.kpasskey.net.pairRequestLine
import org.kpasskey.net.parseDesktopMessage
import org.kpasskey.pair.PairingInvite
import org.kpasskey.store.DesktopStore
import org.kpasskey.store.PairedDesktop

data class ActivityEntry(val summary: String, val outcome: String, val at: Long)

sealed interface PairOutcome {
    data class Paired(val deviceId: String) : PairOutcome

    data class Refused(val reason: String) : PairOutcome

    data object Unreachable : PairOutcome
}

/**
 * Owns the link to the desktop and the request currently awaiting the user.
 *
 * Deliberately knows nothing about notifications or Compose: the service observes
 * [pending] to raise and clear the notification, and the UI observes the same flow to draw
 * the approval screen. That is what lets an approval survive its Activity being destroyed —
 * the request lives here, not in the screen showing it.
 */
class LinkController(
    private val keys: AuthKeyStore,
    private val store: DesktopStore,
) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    private val mutablePending = MutableStateFlow<Approval?>(null)
    val pending: StateFlow<Approval?> = mutablePending.asStateFlow()

    private val mutableState = MutableStateFlow(LinkState.Waiting)
    val state: StateFlow<LinkState> = mutableState.asStateFlow()

    private val mutableActivity = MutableStateFlow<List<ActivityEntry>>(emptyList())
    val activity: StateFlow<List<ActivityEntry>> = mutableActivity.asStateFlow()

    private var link: DesktopLink? = null
    private var job: Job? = null
    private var pairing: CompletableDeferred<DesktopMessage.PairResult>? = null

    /** Brings the link up for the stored desktop, if there is one. Idempotent. */
    fun start() {
        val desktop = store.paired.value ?: return
        connect(desktop.address, desktop.port)
    }

    fun stop() {
        job?.cancel()
        job = null
        link = null
        mutableState.value = LinkState.Waiting
    }

    private fun connect(address: String, port: Int) {
        if (job?.isActive == true) {
            return
        }
        val opened = DesktopLink(address, port)
        link = opened
        job = scope.launch {
            launch { opened.state.collect { mutableState.value = it } }
            opened.run(::onMessage)
        }
    }

    private suspend fun onMessage(line: String) {
        when (val message = parseDesktopMessage(line)) {
            is DesktopMessage.Verify -> present(Approval.Sign(message.request))
            is DesktopMessage.WebAuthnCreate -> present(Approval.CreateCredential(message))
            is DesktopMessage.WebAuthnAssert -> present(Approval.AssertCredential(message))
            is DesktopMessage.PairResult -> pairing?.complete(message)
            null -> Unit
        }
    }

    private fun present(approval: Approval) {
        mutablePending.value = approval
        scope.launch {
            delay(REQUEST_TTL_MS)
            // Only clear if it is still the same request — a second request arriving in the
            // meantime owns the slot and its own expiry.
            if (mutablePending.value?.id == approval.id) {
                mutablePending.value = null
                record(approval.sentence(), "expired")
            }
        }
    }

    /**
     * Sends a signature. For [Approval.Sign] it must cover [VerifyRequest.raw] verbatim; for
     * a WebAuthn assertion it must cover the payload the desktop sent, also verbatim.
     */
    fun approve(approval: Approval, signature: ByteArray) {
        link?.send(approvalLine(approval.id, signature))
        clear(approval, "approved")
    }

    /** Answers a passkey creation with the new credential's id and public key. */
    fun approveCredential(approval: Approval, credentialId: String, publicKeyPem: String) {
        link?.send(credentialLine(approval.id, credentialId, publicKeyPem))
        clear(approval, "created")
    }

    fun deny(approval: Approval, reason: String) {
        link?.send(denialLine(approval.id, reason))
        clear(approval, "denied")
    }

    private fun clear(approval: Approval, outcome: String) {
        if (mutablePending.value?.id == approval.id) {
            mutablePending.value = null
        }
        record(approval.sentence(), outcome)
    }

    private fun record(summary: String, outcome: String) {
        val entry = ActivityEntry(summary, outcome, System.currentTimeMillis())
        mutableActivity.value = (listOf(entry) + mutableActivity.value).take(ACTIVITY_LIMIT)
    }

    /**
     * Opens a link to the address from the QR, sends the pairing request and waits for the
     * desktop's verdict. On success the link stays up as the paired link, so the phone is
     * immediately ready to answer.
     */
    suspend fun pair(invite: PairingInvite, phoneName: String): PairOutcome {
        if (!keys.exists()) {
            return PairOutcome.Refused("no key on this phone")
        }
        stop()
        connect(invite.address, invite.port)

        val awaited = CompletableDeferred<DesktopMessage.PairResult>()
        pairing = awaited
        try {
            link?.send(
                pairRequestLine(
                    token = invite.token,
                    publicKeyPem = keys.publicKeyPem(),
                    name = phoneName,
                    model = android.os.Build.MODEL,
                    securityLevel = keys.summary().securityLevel,
                    // Honest placeholder: the phone cannot establish its own verified-boot
                    // state. It comes from the attestation chain, which the desktop does not
                    // parse yet.
                    verifiedBoot = "unverified",
                ),
            )
            val result = withTimeoutOrNull(PAIR_TIMEOUT_MS) { awaited.await() }
                ?: return PairOutcome.Unreachable
            if (!result.ok) {
                return PairOutcome.Refused(result.reason)
            }
            store.save(
                PairedDesktop(
                    deviceId = result.deviceId,
                    hostName = invite.hostName,
                    address = invite.address,
                    port = invite.port,
                    pairedAt = System.currentTimeMillis(),
                ),
            )
            return PairOutcome.Paired(result.deviceId)
        } finally {
            pairing = null
        }
    }

    private companion object {
        /**
         * The request carries no deadline, so this mirrors the daemon's `--timeout` default.
         * If that default changes, a stale prompt outlives the request it belongs to.
         */
        const val REQUEST_TTL_MS = 120_000L
        const val PAIR_TIMEOUT_MS = 20_000L
        const val ACTIVITY_LIMIT = 20
    }
}
