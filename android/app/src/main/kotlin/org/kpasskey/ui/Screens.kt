package org.kpasskey.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Sync
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import org.kpasskey.R
import org.kpasskey.crypto.AuthKeyStore
import org.kpasskey.link.ActivityEntry
import org.kpasskey.net.LinkState
import org.kpasskey.pair.PairingInvite
import org.kpasskey.store.PairedDesktop
import java.text.DateFormat
import java.util.Date

@Composable
fun WelcomeScreen(onPair: () -> Unit) {
    Column(
        modifier = Modifier.fillMaxSize().safeDrawingPadding().padding(28.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Spacer(Modifier.weight(1f))
        Text(
            stringResource(R.string.welcome_title),
            style = MaterialTheme.typography.headlineMedium,
            textAlign = TextAlign.Center,
        )
        Text(
            stringResource(R.string.welcome_body),
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
            modifier = Modifier.padding(top = 16.dp),
        )
        Spacer(Modifier.weight(1f))
        Button(onClick = onPair, modifier = Modifier.fillMaxWidth()) {
            Text(stringResource(R.string.welcome_action))
        }
    }
}

@Composable
fun ConfirmScreen(
    invite: PairingInvite,
    pairing: Boolean,
    error: String?,
    onConfirm: () -> Unit,
    onCancel: () -> Unit,
) {
    Column(
        modifier = Modifier.fillMaxSize().safeDrawingPadding().padding(28.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Spacer(Modifier.weight(1f))
        Text(
            stringResource(R.string.confirm_title),
            style = MaterialTheme.typography.headlineSmall,
            textAlign = TextAlign.Center,
        )
        Text(
            stringResource(R.string.confirm_body),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
            modifier = Modifier.padding(top = 12.dp),
        )
        Text(
            invite.confirmationCode,
            style = MaterialTheme.typography.displayMedium,
            modifier = Modifier.padding(vertical = 32.dp),
        )
        Text(
            invite.hostName,
            style = MaterialTheme.typography.titleMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        if (error != null) {
            Text(
                error,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.error,
                textAlign = TextAlign.Center,
                modifier = Modifier.padding(top = 16.dp),
            )
        }
        Spacer(Modifier.weight(1f))
        if (pairing) {
            CircularProgressIndicator()
            Text(
                stringResource(R.string.confirm_pairing),
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.padding(top = 12.dp),
            )
        } else {
            Button(onClick = onConfirm, modifier = Modifier.fillMaxWidth()) {
                Text(stringResource(R.string.confirm_action))
            }
            TextButton(onClick = onCancel) { Text("Cancel") }
        }
    }
}

@Composable
fun HomeScreen(
    desktop: PairedDesktop,
    state: LinkState,
    fingerprint: String,
    activity: List<ActivityEntry>,
    warnings: List<Warning>,
    onAdvanced: () -> Unit,
) {
    // Fixed height with the activity list taking the slack, so the screen fills rather than
    // stacking everything at the top over a void.
    Column(
        modifier = Modifier
            .fillMaxSize()
            .safeDrawingPadding()
            .padding(horizontal = 24.dp, vertical = 16.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        warnings.forEach { warning ->
            WarningCard(warning.text, stringResource(R.string.advanced_fix), warning.onFix)
        }

        // The desktop is the subject of this screen. What the phone is, the user can see by
        // holding it; what it is connected to, they cannot.
        StatusCard(
            icon = if (state == LinkState.Connected) Icons.Filled.CheckCircle else Icons.Filled.Sync,
            title = desktop.hostName,
            subtitle = when (state) {
                LinkState.Connected -> stringResource(R.string.status_connected_short)
                LinkState.Connecting -> stringResource(R.string.status_connecting)
                LinkState.Waiting -> stringResource(R.string.status_waiting)
            },
        )

        Text(stringResource(R.string.home_activity), style = MaterialTheme.typography.titleSmall)

        Column(
            modifier = Modifier
                .weight(1f)
                .fillMaxWidth()
                .verticalScroll(rememberScrollState()),
        ) {
            if (activity.isEmpty()) {
                Text(
                    stringResource(R.string.home_no_activity),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            } else {
                activity.forEach { entry ->
                    Column(modifier = Modifier.padding(vertical = 8.dp)) {
                        Text(entry.summary, style = MaterialTheme.typography.bodyMedium)
                        Text(
                            "${entry.outcome} · ${formatTime(entry.at)}",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    HorizontalDivider()
                }
            }
        }

        // Kept because it is the only thing on this phone the desktop can verify — it is how
        // you tell which row in System Settings is this handset. Anchored to the bottom so it
        // reads as a reference, not as the point of the screen.
        Card(modifier = Modifier.fillMaxWidth()) {
            Column(
                modifier = Modifier.fillMaxWidth().padding(16.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                FingerprintCode(fingerprint)
                Text(
                    stringResource(R.string.home_fingerprint_caption),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                    modifier = Modifier.padding(top = 4.dp),
                )
            }
        }

        TextButton(
            onClick = onAdvanced,
            modifier = Modifier.align(Alignment.CenterHorizontally),
        ) { Text(stringResource(R.string.home_advanced)) }
    }
}

@Composable
fun AdvancedScreen(
    summary: AuthKeyStore.Summary?,
    fingerprint: String,
    onExport: () -> Unit,
    onUnpair: () -> Unit,
    onBack: () -> Unit,
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .safeDrawingPadding()
            .verticalScroll(rememberScrollState())
            .padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(stringResource(R.string.advanced_title), style = MaterialTheme.typography.headlineSmall)

        Card(modifier = Modifier.fillMaxWidth()) {
            Column(Modifier.padding(20.dp)) {
                Text(stringResource(R.string.advanced_key), style = MaterialTheme.typography.titleSmall)
                Text(fingerprint, style = MaterialTheme.typography.bodyMedium)
                if (summary != null) {
                    Text(
                        "security level: ${summary.securityLevel}\n" +
                            "StrongBox: ${summary.strongBoxBacked}\n" +
                            "auth required: ${summary.userAuthenticationRequired}\n" +
                            "invalidate on enrol: ${summary.invalidatedByBiometricEnrollment}\n" +
                            "attestation certs: ${summary.chainLength}",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(top = 8.dp),
                    )
                }
            }
        }

        TextButton(onClick = onExport) { Text(stringResource(R.string.advanced_export)) }
        TextButton(onClick = onUnpair) {
            Text(
                stringResource(R.string.advanced_unpair),
                color = MaterialTheme.colorScheme.error,
            )
        }
        Spacer(Modifier.weight(1f))
        TextButton(onClick = onBack) { Text("Back") }
    }
}

@Composable
fun KeyInvalidatedScreen(onRepair: () -> Unit) {
    Column(
        modifier = Modifier.fillMaxSize().safeDrawingPadding().padding(28.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            stringResource(R.string.key_invalidated_title),
            style = MaterialTheme.typography.headlineSmall,
            textAlign = TextAlign.Center,
        )
        Text(
            stringResource(R.string.key_invalidated_body),
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
            modifier = Modifier.padding(top = 16.dp),
        )
        Button(onClick = onRepair, modifier = Modifier.padding(top = 32.dp)) {
            Text(stringResource(R.string.key_invalidated_action))
        }
    }
}

data class Warning(val text: String, val onFix: () -> Unit)

private fun formatTime(at: Long): String =
    DateFormat.getTimeInstance(DateFormat.SHORT).format(Date(at))
