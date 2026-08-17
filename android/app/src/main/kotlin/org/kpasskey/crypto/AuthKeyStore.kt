package org.kpasskey.crypto

import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyInfo
import android.security.keystore.KeyProperties
import android.security.keystore.StrongBoxUnavailableException
import java.security.KeyFactory
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.MessageDigest
import java.security.PrivateKey
import java.security.PublicKey
import java.security.SecureRandom
import java.security.Signature
import java.security.cert.Certificate
import java.security.spec.ECGenParameterSpec
import java.util.Base64

/**
 * The authentication key: EC P-256, non-extractable, and usable only after a fresh
 * BIOMETRIC_STRONG match. Every constraint below is verified by the desktop at pairing time
 * by parsing the attestation certificate, so weakening any of them breaks pairing rather
 * than silently degrading security.
 */
class AuthKeyStore private constructor(private val keyStore: KeyStore) {

    data class Summary(
        val securityLevel: String,
        val strongBoxBacked: Boolean,
        val userAuthenticationRequired: Boolean,
        val invalidatedByBiometricEnrollment: Boolean,
        val chainLength: Int,
    )

    fun exists(): Boolean = keyStore.containsAlias(ALIAS)

    fun delete() {
        if (exists()) {
            keyStore.deleteEntry(ALIAS)
        }
    }

    /**
     * Generates a fresh key bound to [challenge], which the desktop echoes back out of the
     * attestation extension to prove the key was made for this pairing and no other.
     * Falls back to the TEE when the device has no StrongBox.
     */
    fun generate(challenge: ByteArray): Summary {
        delete()
        try {
            generateInternal(challenge, strongBox = true)
        } catch (_: StrongBoxUnavailableException) {
            generateInternal(challenge, strongBox = false)
        }
        return summary()
    }

    private fun generateInternal(challenge: ByteArray, strongBox: Boolean) {
        val builder =
            KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_SIGN)
                .setAlgorithmParameterSpec(ECGenParameterSpec(CURVE))
                .setDigests(KeyProperties.DIGEST_SHA256)
                .setUserAuthenticationRequired(true)
                // Timeout 0 means auth-per-use: one fingerprint touch authorises exactly one
                // signature, never a window of time.
                .setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG)
                .setInvalidatedByBiometricEnrollment(true)
                .setUnlockedDeviceRequired(true)
                .setAttestationChallenge(challenge)
        if (strongBox) {
            builder.setIsStrongBoxBacked(true)
        }

        val generator = KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC, PROVIDER)
        generator.initialize(builder.build())
        generator.generateKeyPair()
    }

    fun summary(): Summary {
        val key = privateKey()
        val info = KeyFactory.getInstance(key.algorithm, PROVIDER).getKeySpec(key, KeyInfo::class.java)
        val level = info.securityLevel
        return Summary(
            securityLevel = securityLevelName(level),
            strongBoxBacked = level == KeyProperties.SECURITY_LEVEL_STRONGBOX,
            userAuthenticationRequired = info.isUserAuthenticationRequired,
            invalidatedByBiometricEnrollment = info.isInvalidatedByBiometricEnrollment,
            chainLength = chain().size,
        )
    }

    /**
     * Prepares a signing operation for [androidx.biometric.BiometricPrompt.CryptoObject].
     *
     * Throws `KeyPermanentlyInvalidatedException` when a new biometric has been enrolled
     * since the key was created — the signal the desktop needs to force re-pairing.
     */
    fun prepareSignature(): Signature =
        Signature.getInstance(SIGNATURE_ALGORITHM).apply { initSign(privateKey()) }

    fun publicKey(): PublicKey = keyStore.getCertificate(ALIAS).publicKey

    /**
     * The same value the desktop shows beside this phone in System Settings: 48 bits of
     * SHA-256 over the key's X.509 SPKI, which is byte-identical to what `publicKey().encoded`
     * produces here and what the desktop re-encodes from the stored PEM.
     */
    fun fingerprint(): String {
        val digest = MessageDigest.getInstance("SHA-256").digest(publicKey().encoded)
        return digest.take(FINGERPRINT_BYTES)
            .joinToString(separator = "") { byte -> "%02X".format(byte) }
            .chunked(FINGERPRINT_GROUP)
            .joinToString(separator = " ")
    }

    /** X.509 SPKI in PEM, which is what the desktop parses and stores at pairing. */
    fun publicKeyPem(): String = toPublicKeyPem(publicKey())

    /**
     * A fresh WebAuthn credential key for one site, under the same biometric gate as the
     * pairing key. The private half never leaves the TEE, so a site's assertion can only be
     * produced on this phone, by this fingerprint.
     *
     * No attestation challenge: registration uses `none` attestation, so there is nothing
     * for a challenge to be bound into.
     */
    fun createCredential(): Credential {
        val raw = ByteArray(CREDENTIAL_ID_LENGTH).also(SecureRandom()::nextBytes)
        val credentialId = Base64.getUrlEncoder().withoutPadding().encodeToString(raw)

        val specification =
            KeyGenParameterSpec.Builder(credentialAlias(credentialId), KeyProperties.PURPOSE_SIGN)
                .setAlgorithmParameterSpec(ECGenParameterSpec(CURVE))
                .setDigests(KeyProperties.DIGEST_SHA256)
                .setUserAuthenticationRequired(true)
                .setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG)
                .setInvalidatedByBiometricEnrollment(true)
                .setUnlockedDeviceRequired(true)
                .build()

        val generator = KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC, PROVIDER)
        generator.initialize(specification)
        val pair = generator.generateKeyPair()

        return Credential(credentialId, toPublicKeyPem(pair.public))
    }

    /** Throws if the site names a credential this phone does not hold. */
    fun prepareCredentialSignature(credentialId: String): Signature {
        val key =
            keyStore.getKey(credentialAlias(credentialId), null) as? PrivateKey
                ?: throw IllegalStateException("no credential $credentialId on this phone")
        return Signature.getInstance(SIGNATURE_ALGORITHM).apply { initSign(key) }
    }

    data class Credential(val credentialId: String, val publicKeyPem: String)

    private fun credentialAlias(credentialId: String): String = "$CREDENTIAL_PREFIX$credentialId"

    private fun toPublicKeyPem(key: PublicKey): String {
        val encoder = Base64.getMimeEncoder(PEM_LINE_LENGTH, "\n".toByteArray())
        return buildString {
            append("-----BEGIN PUBLIC KEY-----\n")
            append(encoder.encodeToString(key.encoded))
            append("\n-----END PUBLIC KEY-----\n")
        }
    }

    fun chain(): List<Certificate> = keyStore.getCertificateChain(ALIAS)?.toList().orEmpty()

    fun chainPem(): String = chain().joinToString(separator = "") { certificate -> toPem(certificate) }

    private fun privateKey(): PrivateKey =
        keyStore.getKey(ALIAS, null) as? PrivateKey
            ?: throw IllegalStateException("no authentication key; generate one first")

    private fun toPem(certificate: Certificate): String {
        val encoder = Base64.getMimeEncoder(PEM_LINE_LENGTH, "\n".toByteArray())
        return buildString {
            append("-----BEGIN CERTIFICATE-----\n")
            append(encoder.encodeToString(certificate.encoded))
            append("\n-----END CERTIFICATE-----\n")
        }
    }

    private fun securityLevelName(level: Int): String =
        when (level) {
            KeyProperties.SECURITY_LEVEL_STRONGBOX -> "StrongBox"
            KeyProperties.SECURITY_LEVEL_TRUSTED_ENVIRONMENT -> "TrustedEnvironment"
            KeyProperties.SECURITY_LEVEL_SOFTWARE -> "Software (REJECTED at pairing)"
            KeyProperties.SECURITY_LEVEL_UNKNOWN_SECURE -> "Unknown but secure"
            else -> "Unknown"
        }

    companion object {
        const val ALIAS: String = "kpasskey.auth.v1"
        const val SIGNATURE_ALGORITHM: String = "SHA256withECDSA"

        private const val PROVIDER = "AndroidKeyStore"
        private const val CURVE = "secp256r1"
        private const val PEM_LINE_LENGTH = 64
        private const val FINGERPRINT_BYTES = 6
        private const val FINGERPRINT_GROUP = 4
        private const val CREDENTIAL_PREFIX = "kpasskey.cred."
        private const val CREDENTIAL_ID_LENGTH = 32

        fun open(): AuthKeyStore {
            val keyStore = KeyStore.getInstance(PROVIDER)
            keyStore.load(null)
            return AuthKeyStore(keyStore)
        }
    }
}
