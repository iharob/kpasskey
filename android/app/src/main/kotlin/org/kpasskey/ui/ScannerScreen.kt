package org.kpasskey.ui

import android.Manifest
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.MultiFormatReader
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.common.HybridBinarizer
import org.kpasskey.R
import org.kpasskey.pair.PairingInvite
import org.kpasskey.pair.parsePairingUri

@Composable
fun ScannerScreen(onScanned: (PairingInvite) -> Unit) {
    val context = LocalContext.current
    var granted by remember {
        mutableStateOf(
            ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) ==
                PackageManager.PERMISSION_GRANTED,
        )
    }
    val request =
        rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { allowed ->
            granted = allowed
        }

    if (!granted) {
        Column(
            modifier = Modifier.fillMaxSize().padding(32.dp),
            verticalArrangement = androidx.compose.foundation.layout.Arrangement.Center,
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Text(
                stringResource(R.string.scan_permission),
                style = MaterialTheme.typography.bodyLarge,
                textAlign = TextAlign.Center,
            )
            Button(
                onClick = { request.launch(Manifest.permission.CAMERA) },
                modifier = Modifier.padding(top = 24.dp),
            ) { Text(stringResource(R.string.scan_grant)) }
        }
        return
    }

    Box(modifier = Modifier.fillMaxSize()) {
        CameraPreview(onDecoded = onScanned)

        Box(
            modifier = Modifier
                .align(Alignment.Center)
                .size(260.dp)
                .border(3.dp, Color.White.copy(alpha = 0.9f), RoundedCornerShape(24.dp)),
        )

        Text(
            text = stringResource(R.string.scan_help),
            modifier = Modifier
                .align(Alignment.BottomCenter)
                .safeDrawingPadding()
                .fillMaxWidth()
                .padding(32.dp),
            color = Color.White,
            style = MaterialTheme.typography.bodyMedium,
            textAlign = TextAlign.Center,
        )
    }
}

@Composable
private fun CameraPreview(onDecoded: (PairingInvite) -> Unit) {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current
    val handler by rememberUpdatedState(onDecoded)
    // Latches so a QR held in front of the camera decodes once rather than on every frame.
    val delivered = remember { java.util.concurrent.atomic.AtomicBoolean(false) }
    val executor = remember { java.util.concurrent.Executors.newSingleThreadExecutor() }
    val view = remember { PreviewView(context) }

    DisposableEffect(Unit) {
        val future = ProcessCameraProvider.getInstance(context)
        future.addListener({
            val provider = future.get()
            val preview = Preview.Builder().build().apply { surfaceProvider = view.surfaceProvider }
            val analysis =
                ImageAnalysis.Builder()
                    .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                    .build()
            analysis.setAnalyzer(executor) { image ->
                val text = decode(image)
                image.close()
                val invite = text?.let(::parsePairingUri)
                if (invite != null && delivered.compareAndSet(false, true)) {
                    view.post { handler(invite) }
                }
            }
            provider.unbindAll()
            provider.bindToLifecycle(lifecycleOwner, CameraSelector.DEFAULT_BACK_CAMERA, preview, analysis)
        }, ContextCompat.getMainExecutor(context))

        onDispose {
            runCatching { ProcessCameraProvider.getInstance(context).get().unbindAll() }
            executor.shutdown()
        }
    }

    AndroidView(factory = { view }, modifier = Modifier.fillMaxSize())
}

/**
 * The Y plane alone is the luminance channel, which is all a QR decoder needs.
 * `rowStride` rather than `width` is the buffer's real pitch — using width here decodes
 * garbage on devices that pad rows.
 */
private fun decode(image: ImageProxy): String? {
    val plane = image.planes.firstOrNull() ?: return null
    val buffer = plane.buffer
    val bytes = ByteArray(buffer.remaining())
    buffer.get(bytes)

    val source =
        PlanarYUVLuminanceSource(
            bytes,
            plane.rowStride,
            image.height,
            0,
            0,
            image.width,
            image.height,
            false,
        )
    val reader =
        MultiFormatReader().apply {
            setHints(mapOf(DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE)))
        }
    return runCatching { reader.decode(BinaryBitmap(HybridBinarizer(source))).text }.getOrNull()
}
