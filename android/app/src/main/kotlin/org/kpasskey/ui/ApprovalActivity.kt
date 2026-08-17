package org.kpasskey.ui

import android.app.KeyguardManager
import android.content.Context
import android.content.Intent
import android.os.Bundle
import androidx.activity.compose.setContent
import androidx.appcompat.app.AppCompatActivity
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.flow.MutableStateFlow
import java.util.Base64
import org.kpasskey.R
import org.kpasskey.biometric.REASON_INVALIDATED
import org.kpasskey.biometric.promptAndSign
import org.kpasskey.biometric.promptAndSignWithCredential
import org.kpasskey.biometric.promptForConsent
import org.kpasskey.container
import org.kpasskey.net.Approval
import org.kpasskey.ui.theme.KpkTheme

/**
 * The screen a notification tap or full-screen intent lands on.
 *
 * `AppCompatActivity` rather than `ComponentActivity` because androidx `BiometricPrompt`
 * requires a `FragmentActivity` host.
 */
class ApprovalActivity : AppCompatActivity() {

    /** Recomposes the Approve button the moment the keyguard actually goes away. */
    private val locked = MutableStateFlow(true)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Belt and braces alongside the manifest attributes, which apply before first draw.
        setShowWhenLocked(true)
        setTurnScreenOn(true)

        val controller = container.link
        val keys = container.keys

        setContent {
            KpkTheme {
                val pending by controller.pending.collectAsStateWithLifecycle()
                val isLocked by locked.collectAsStateWithLifecycle()
                Surface(modifier = Modifier.fillMaxSize()) {
                    val request = pending
                    if (request == null) {
                        NothingWaiting(onDismiss = ::finish)
                    } else {
                        ApprovalScreen(
                            headline = request.sentence(),
                            detail = request.detail(),
                            locked = isLocked,
                            onUnlock = ::dismissKeyguard,
                            onApprove = { answer(request) },
                            onDeny = {
                                controller.deny(request, "user-cancelled")
                                finish()
                            },
                        )
                    }
                }
            }
        }
    }

    /** Each request kind needs a different act from the user, but the same fingerprint. */
    private fun answer(approval: Approval) {
        val controller = container.link
        val keys = container.keys
        val onFailed = { reason: String ->
            if (reason == REASON_INVALIDATED) {
                controller.deny(approval, reason)
                finish()
            }
        }

        when (approval) {
            is Approval.Sign ->
                promptAndSign(
                    activity = this,
                    keys = keys,
                    title = approval.sentence(),
                    subtitle = getString(R.string.action_approve),
                    payload = approval.request.raw.toByteArray(Charsets.UTF_8),
                    onSigned = { signature ->
                        controller.approve(approval, signature)
                        finish()
                    },
                    onFailed = onFailed,
                )

            is Approval.CreateCredential ->
                // Nothing to sign at registration — `none` attestation carries no signature —
                // so the fingerprint is consent, and the key it creates is what gets used later.
                promptForConsent(
                    activity = this,
                    title = approval.sentence(),
                    subtitle = approval.detail(),
                    onVerified = {
                        runCatching { keys.createCredential() }
                            .onSuccess { credential ->
                                controller.approveCredential(
                                    approval,
                                    credential.credentialId,
                                    credential.publicKeyPem,
                                )
                                finish()
                            }
                            .onFailure {
                                controller.deny(approval, "create-failed")
                                finish()
                            }
                    },
                    onFailed = onFailed,
                )

            is Approval.AssertCredential ->
                promptAndSignWithCredential(
                    activity = this,
                    keys = keys,
                    credentialId = approval.message.credentialId,
                    title = approval.sentence(),
                    subtitle = getString(R.string.action_approve),
                    payload = Base64.getDecoder().decode(approval.message.payload),
                    onSigned = { signature ->
                        controller.approve(approval, signature)
                        finish()
                    },
                    onFailed = onFailed,
                )
        }
    }

    override fun onResume() {
        super.onResume()
        refreshLocked()
    }

    private fun refreshLocked() {
        locked.value = getSystemService(KeyguardManager::class.java)?.isDeviceLocked == true
    }

    /**
     * The key is `setUnlockedDeviceRequired`, so its blob cannot even be unwrapped while the
     * keyguard is up. This asks for the normal unlock rather than pretending to approve.
     */
    private fun dismissKeyguard() {
        val keyguard = getSystemService(KeyguardManager::class.java) ?: return
        keyguard.requestDismissKeyguard(
            this,
            object : KeyguardManager.KeyguardDismissCallback() {
                override fun onDismissSucceeded() = refreshLocked()

                override fun onDismissCancelled() = refreshLocked()
            },
        )
    }

    companion object {
        fun intent(context: Context): Intent =
            Intent(context, ApprovalActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
    }
}

@Composable
private fun ApprovalScreen(
    headline: String,
    detail: String,
    locked: Boolean,
    onUnlock: () -> Unit,
    onApprove: () -> Unit,
    onDeny: () -> Unit,
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .safeDrawingPadding()
            .padding(28.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Spacer(Modifier.weight(1f))

        Text(
            text = headline,
            style = MaterialTheme.typography.headlineMedium,
            textAlign = TextAlign.Center,
        )

        if (detail.isNotBlank()) {
            Card(modifier = Modifier.fillMaxWidth().padding(top = 24.dp)) {
                Column(Modifier.padding(16.dp)) {
                    Text(
                        stringResource(R.string.approval_detail),
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Text(detail, style = MaterialTheme.typography.bodyMedium)
                }
            }
        }

        Spacer(Modifier.weight(1f))

        Column(
            modifier = Modifier.fillMaxWidth(),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            if (locked) {
                Button(onClick = onUnlock, modifier = Modifier.fillMaxWidth()) {
                    Text(stringResource(R.string.approval_unlock_first))
                }
            } else {
                Button(onClick = onApprove, modifier = Modifier.fillMaxWidth()) {
                    Text(stringResource(R.string.action_approve))
                }
            }
            TextButton(onClick = onDeny) { Text(stringResource(R.string.action_deny)) }
        }
    }
}

@Composable
private fun NothingWaiting(onDismiss: () -> Unit) {
    Column(
        modifier = Modifier.fillMaxSize().safeDrawingPadding().padding(28.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            stringResource(R.string.approval_gone),
            style = MaterialTheme.typography.titleMedium,
            textAlign = TextAlign.Center,
        )
        TextButton(onClick = onDismiss, modifier = Modifier.padding(top = 16.dp)) { Text("Close") }
    }
}
