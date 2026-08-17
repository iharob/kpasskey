package org.kpasskey.biometric

import android.security.keystore.KeyPermanentlyInvalidatedException
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.fragment.app.FragmentActivity
import org.kpasskey.crypto.AuthKeyStore

/**
 * Shows the fingerprint prompt and signs [payload] with the key it unlocks.
 *
 * [payload] must be the bytes the desktop sent, verbatim. A `Signature` is never cached
 * across a prompt: one biometric touch authorises exactly one signing operation, and reusing
 * one would be reusing the user's consent.
 */
fun promptAndSign(
    activity: FragmentActivity,
    keys: AuthKeyStore,
    title: String,
    subtitle: String,
    payload: ByteArray,
    onSigned: (ByteArray) -> Unit,
    onFailed: (String) -> Unit,
) {
    val signature =
        try {
            keys.prepareSignature()
        } catch (_: KeyPermanentlyInvalidatedException) {
            onFailed(REASON_INVALIDATED)
            return
        } catch (_: Exception) {
            // Also the locked-device case: the key blob cannot be unwrapped while the
            // keyguard is up, and that surfaces here rather than at sign time.
            onFailed(REASON_UNAVAILABLE)
            return
        }

    authenticateAndSign(activity, signature, title, subtitle, payload, onSigned, onFailed)
}

private fun authenticateAndSign(
    activity: FragmentActivity,
    signature: java.security.Signature,
    title: String,
    subtitle: String,
    payload: ByteArray,
    onSigned: (ByteArray) -> Unit,
    onFailed: (String) -> Unit,
) {
    val prompt =
        BiometricPrompt(
            activity,
            activity.mainExecutor,
            object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                    val bound = result.cryptoObject?.signature
                    if (bound == null) {
                        onFailed(REASON_NO_CRYPTO_OBJECT)
                        return
                    }
                    runCatching {
                        bound.update(payload)
                        bound.sign()
                    }
                        .onSuccess(onSigned)
                        .onFailure { onFailed(REASON_SIGN_FAILED) }
                }

                override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                    onFailed(REASON_CANCELLED)
                }
            },
        )

    val info =
        BiometricPrompt.PromptInfo.Builder()
            .setTitle(title)
            .setSubtitle(subtitle)
            .setNegativeButtonText("Cancel")
            // No DEVICE_CREDENTIAL: a PIN fallback would break the CryptoObject binding and
            // defeat the whole point of the key.
            .setAllowedAuthenticators(BiometricManager.Authenticators.BIOMETRIC_STRONG)
            .build()

    prompt.authenticate(info, BiometricPrompt.CryptoObject(signature))
}

/** User verification with nothing to sign — passkey creation has no signature to bind. */
fun promptForConsent(
    activity: FragmentActivity,
    title: String,
    subtitle: String,
    onVerified: () -> Unit,
    onFailed: (String) -> Unit,
) {
    val prompt =
        BiometricPrompt(
            activity,
            activity.mainExecutor,
            object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) =
                    onVerified()

                override fun onAuthenticationError(errorCode: Int, errString: CharSequence) =
                    onFailed(REASON_CANCELLED)
            },
        )
    prompt.authenticate(
        BiometricPrompt.PromptInfo.Builder()
            .setTitle(title)
            .setSubtitle(subtitle)
            .setNegativeButtonText("Cancel")
            .setAllowedAuthenticators(BiometricManager.Authenticators.BIOMETRIC_STRONG)
            .build(),
    )
}

/** Signs [payload] with a per-site WebAuthn credential key. */
fun promptAndSignWithCredential(
    activity: FragmentActivity,
    keys: AuthKeyStore,
    credentialId: String,
    title: String,
    subtitle: String,
    payload: ByteArray,
    onSigned: (ByteArray) -> Unit,
    onFailed: (String) -> Unit,
) {
    val signature =
        try {
            keys.prepareCredentialSignature(credentialId)
        } catch (_: KeyPermanentlyInvalidatedException) {
            onFailed(REASON_INVALIDATED)
            return
        } catch (_: Exception) {
            onFailed(REASON_UNAVAILABLE)
            return
        }
    authenticateAndSign(activity, signature, title, subtitle, payload, onSigned, onFailed)
}

const val REASON_INVALIDATED = "key-invalidated"
const val REASON_UNAVAILABLE = "prepare-failed"
const val REASON_NO_CRYPTO_OBJECT = "no-crypto-object"
const val REASON_SIGN_FAILED = "sign-failed"
const val REASON_CANCELLED = "user-cancelled"
