package org.kpasskey.ui

import android.Manifest
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.res.stringResource
import androidx.core.app.NotificationManagerCompat
import androidx.core.net.toUri
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.launch
import org.kpasskey.R
import org.kpasskey.container
import org.kpasskey.link.PairOutcome
import org.kpasskey.pair.PairingInvite
import org.kpasskey.service.LinkService
import org.kpasskey.service.Notifications
import org.kpasskey.ui.theme.KpkTheme
import java.io.File
import java.util.Base64

private sealed interface Screen {
    data object Welcome : Screen

    data object Scan : Screen

    data class Confirm(val invite: PairingInvite) : Screen

    data object Home : Screen

    data object Advanced : Screen
}

class MainActivity : AppCompatActivity() {

    private val notificationPermission =
        registerForActivityResult(ActivityResultContracts.RequestPermission()) { }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            notificationPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
        if (container.desktops.paired.value != null) {
            LinkService.start(this)
        }

        setContent {
            KpkTheme {
                Surface(modifier = Modifier.fillMaxSize()) { Root() }
            }
        }
    }

    @Composable
    private fun Root() {
        val scope = rememberCoroutineScope()
        val container = container
        val desktop by container.desktops.paired.collectAsStateWithLifecycle()
        val linkState by container.link.state.collectAsStateWithLifecycle()
        val phoneName by container.desktops.phoneName.collectAsStateWithLifecycle()
        val activity by container.link.activity.collectAsStateWithLifecycle()

        var screen by remember { mutableStateOf<Screen>(Screen.Welcome) }
        var pairing by remember { mutableStateOf(false) }
        var pairError by remember { mutableStateOf<String?>(null) }

        val fingerprint =
            remember(desktop) {
                if (container.keys.exists()) {
                    runCatching { container.keys.fingerprint() }.getOrDefault("")
                } else {
                    ""
                }
            }

        // Home and Welcome are decided by whether a desktop is paired, not by navigation
        // history — that way pairing and unpairing land on the right screen by themselves.
        val current =
            when {
                screen is Screen.Scan || screen is Screen.Confirm || screen is Screen.Advanced -> screen
                desktop != null -> Screen.Home
                else -> Screen.Welcome
            }

        when (current) {
            Screen.Welcome -> WelcomeScreen(onPair = { screen = Screen.Scan })

            Screen.Scan ->
                ScannerScreen(
                    onScanned = { invite ->
                        pairError = null
                        screen = Screen.Confirm(invite)
                    },
                )

            is Screen.Confirm -> {
                val invite = current.invite
                ConfirmScreen(
                    invite = invite,
                    pairing = pairing,
                    error = pairError,
                    onCancel = { screen = Screen.Welcome },
                    onConfirm = {
                        pairing = true
                        pairError = null
                        scope.launch {
                            val outcome = pairWith(invite, phoneName)
                            pairing = false
                            when (outcome) {
                                is PairOutcome.Paired -> {
                                    LinkService.start(this@MainActivity)
                                    screen = Screen.Home
                                }
                                is PairOutcome.Refused -> pairError = outcome.reason
                                PairOutcome.Unreachable ->
                                    pairError = "The computer did not answer."
                            }
                        }
                    },
                )
            }

            Screen.Home ->
                desktop?.let { paired ->
                    HomeScreen(
                        desktop = paired,
                        state = linkState,
                        fingerprint = fingerprint,
                        activity = activity,
                        warnings = warnings(),
                        onAdvanced = { screen = Screen.Advanced },
                    )
                }

            Screen.Advanced ->
                AdvancedScreen(
                    summary = remember { runCatching { container.keys.summary() }.getOrNull() },
                    fingerprint = fingerprint,
                    onExport = ::exportChain,
                    onUnpair = {
                        container.link.stop()
                        container.keys.delete()
                        container.desktops.forget()
                        stopService(Intent(this, LinkService::class.java))
                        screen = Screen.Welcome
                    },
                    onBack = { screen = Screen.Home },
                )
        }
    }

    /**
     * The key is generated here, not earlier: the attestation challenge must be the pairing
     * token, which binds the key to this pairing and no other.
     */
    private suspend fun pairWith(invite: PairingInvite, phoneName: String): PairOutcome {
        val token =
            runCatching { Base64.getUrlDecoder().decode(invite.token) }.getOrNull()
                ?: return PairOutcome.Refused("unreadable pairing token")
        runCatching { container.keys.generate(token) }
            .onFailure { error ->
                return PairOutcome.Refused(error.message ?: "could not create a key")
            }
        return container.link.pair(invite, phoneName)
    }

    @Composable
    private fun warnings(): List<Warning> {
        val notifications = remember { Notifications(this) }
        val enabled = NotificationManagerCompat.from(this).areNotificationsEnabled()
        val fullScreen = notifications.canUseFullScreenIntent()

        return buildList {
            if (!enabled) {
                add(
                    Warning(stringResource(R.string.advanced_notifications_off)) {
                        startActivity(
                            Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
                                .putExtra(Settings.EXTRA_APP_PACKAGE, packageName),
                        )
                    },
                )
            }
            if (!fullScreen) {
                add(
                    Warning(stringResource(R.string.advanced_fsi_off)) {
                        startActivity(
                            Intent(
                                Settings.ACTION_MANAGE_APP_USE_FULL_SCREEN_INTENT,
                                "package:$packageName".toUri(),
                            ),
                        )
                    },
                )
            }
        }
    }

    private fun exportChain() {
        runCatching {
            val target = File(getExternalFilesDir(null), "attestation-chain.pem")
            target.writeText(container.keys.chainPem())
        }
    }
}
