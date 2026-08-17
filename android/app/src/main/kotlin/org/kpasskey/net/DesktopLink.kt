package org.kpasskey.net

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.BufferedReader
import java.io.InputStreamReader
import java.io.OutputStream
import java.net.InetSocketAddress
import java.net.Socket

enum class LinkState {
    Connecting,
    Connected,
    Waiting,
}

/**
 * Spike transport: a plain TCP line protocol reached over `adb reverse`. The real client
 * uses TLS 1.3 with SPKI pinning; nothing here is a security boundary, which is why the
 * signature covers the whole request rather than relying on the channel.
 */
class DesktopLink(private val host: String, private val port: Int) {

    private val mutableState = MutableStateFlow(LinkState.Connecting)
    val state: StateFlow<LinkState> = mutableState.asStateFlow()

    /**
     * Unbounded so a reply can be queued while the link is down. A failed write puts the
     * line back: the desktop routes replies by request id rather than by socket, so an
     * assertion the user already approved with their finger survives the link dropping
     * underneath it — which the adb-tunnelled transport does regularly.
     */
    private val outbox = Channel<String>(Channel.UNLIMITED)

    fun send(line: String) {
        outbox.trySend(line)
    }

    /** Reconnects until the calling scope is cancelled. */
    suspend fun run(onMessage: suspend (String) -> Unit): Nothing {
        while (true) {
            mutableState.value = LinkState.Connecting
            try {
                session(onMessage)
            } catch (cancellation: CancellationException) {
                throw cancellation
            } catch (_: Exception) {
                // Every failure here is the same failure — the desktop went away. Reporting
                // which socket call noticed first would tell the user nothing.
            }
            mutableState.value = LinkState.Waiting
            delay(RECONNECT_DELAY_MS)
        }
    }

    private suspend fun session(onMessage: suspend (String) -> Unit) {
        withContext(Dispatchers.IO) {
            Socket().use { socket ->
                socket.tcpNoDelay = true
                socket.connect(InetSocketAddress(host, port), CONNECT_TIMEOUT_MS)
                mutableState.value = LinkState.Connected

                coroutineScope {
                    // A blocked read does not observe cancellation, so cancelling the scope
                    // alone would leave this coroutine parked in readLine() forever. Closing
                    // the socket is what actually breaks it.
                    val closer = launch {
                        try {
                            awaitCancellation()
                        } finally {
                            runCatching { socket.close() }
                        }
                    }
                    val writer = launch { pump(socket.getOutputStream()) }
                    val reader =
                        BufferedReader(InputStreamReader(socket.getInputStream(), Charsets.UTF_8))
                    while (true) {
                        val line = reader.readLine() ?: break
                        if (line.isNotBlank()) {
                            onMessage(line)
                        }
                    }
                    writer.cancel()
                    closer.cancel()
                }
            }
        }
    }

    private suspend fun pump(output: OutputStream) {
        for (line in outbox) {
            try {
                output.write("$line\n".toByteArray(Charsets.UTF_8))
                output.flush()
            } catch (error: Exception) {
                outbox.trySend(line)
                throw error
            }
        }
    }

    private companion object {
        const val CONNECT_TIMEOUT_MS = 4000
        const val RECONNECT_DELAY_MS = 1500L
    }
}
