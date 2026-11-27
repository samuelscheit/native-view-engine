package dev.nve.compose

import android.os.Bundle
import android.os.Handler
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.BasicText
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.unit.dp
import java.util.concurrent.Executor

/**
 * Reusable Compose host Activity: shows one NVE runtime surface for the bundled
 * `libnve_app.so`, an explicit app library ([EXTRA_APP_PATH], hot-reload builds), or a paired
 * `nve pair` server ([EXTRA_PAIR_CONNECT] / [EXTRA_PAIR_TOKEN]).
 */
open class NveHostActivity : ComponentActivity() {
    private var controller: NveSurfaceController? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val metrics = resources.displayMetrics
        val density = if (metrics.density > 0f) metrics.density else 1f
        val main = Handler(Looper.getMainLooper())
        val host = NativeRuntimeHost(1, metrics.widthPixels / density, metrics.heightPixels / density, density, Executor { main.post(it) })
        val c = NveSurfaceController(host)
        controller = c
        c.start(
            sourceOf(
                appPath = intent.getStringExtra(EXTRA_APP_PATH),
                pairUri = intent.getStringExtra(EXTRA_PAIR_CONNECT),
                pairToken = intent.getStringExtra(EXTRA_PAIR_TOKEN),
                bundledApp = applicationInfo.nativeLibraryDir + "/libnve_app.so",
            ),
        )
        setContent {
            Box(Modifier.fillMaxSize()) {
                NveSurface(c, Modifier.fillMaxSize())
                c.error?.let { BasicText(it, Modifier.padding(20.dp), style = TextStyle(color = Color.White)) }
            }
        }
    }

    override fun onDestroy() {
        controller?.close()
        controller = null
        super.onDestroy()
    }

    companion object {
        const val EXTRA_APP_PATH = "nve_app_path"
        const val EXTRA_PAIR_CONNECT = "pair_connect"
        const val EXTRA_PAIR_TOKEN = "pair_token"

        /** Pairing wins over an explicit app path, which wins over the bundled app. */
        fun sourceOf(appPath: String?, pairUri: String?, pairToken: String?, bundledApp: String): NveSource {
            val uri = pairUri?.trim().orEmpty()
            if (uri.isNotEmpty()) return NveSource.Paired(uri, pairToken?.trim().orEmpty())
            val path = appPath?.trim().orEmpty()
            return NveSource.App(path.ifEmpty { bundledApp })
        }
    }
}
