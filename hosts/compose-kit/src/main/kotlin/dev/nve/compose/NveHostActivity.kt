package dev.nve.compose

import android.content.Intent
import android.content.res.Configuration
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier

/**
 * Configuration describing the root NVE surface hosted by an [NveHostActivity].
 */
data class NveHostConfig(
    val surfaceId: Int = DEFAULT_SURFACE_ID,
    val componentName: String = DEFAULT_COMPONENT_NAME,
    val vtableSymbol: String = DEFAULT_VTABLE_SYMBOL,
    val backend: NveRenderBackend = NveRenderBackend.FABRIC,
    val initialLibraryPath: String? = null,
    val detachOnStop: Boolean = true,
) {
    companion object {
        const val DEFAULT_SURFACE_ID: Int = 1
        const val DEFAULT_COMPONENT_NAME: String = "NveRoot"
        const val DEFAULT_VTABLE_SYMBOL: String = "nve_get_interface"

        const val EXTRA_SURFACE_ID: String = "dev.nve.compose.EXTRA_SURFACE_ID"
        const val EXTRA_COMPONENT_NAME: String = "dev.nve.compose.EXTRA_COMPONENT_NAME"
        const val EXTRA_VTABLE_SYMBOL: String = "dev.nve.compose.EXTRA_VTABLE_SYMBOL"
        const val EXTRA_LIBRARY_PATH: String = "dev.nve.compose.EXTRA_LIBRARY_PATH"

        fun fromIntent(intent: Intent?): NveHostConfig {
            if (intent == null) return NveHostConfig()
            val surfaceId = intent.getIntExtra(EXTRA_SURFACE_ID, DEFAULT_SURFACE_ID)
            val componentName = intent.getStringExtra(EXTRA_COMPONENT_NAME)
                ?.takeIf { it.isNotBlank() }
                ?: DEFAULT_COMPONENT_NAME
            val vtableSymbol = intent.getStringExtra(EXTRA_VTABLE_SYMBOL)
                ?.takeIf { it.isNotBlank() }
                ?: DEFAULT_VTABLE_SYMBOL
            val libPath = intent.getStringExtra(EXTRA_LIBRARY_PATH)?.takeIf { it.isNotBlank() }
            return NveHostConfig(
                surfaceId = surfaceId,
                componentName = componentName,
                vtableSymbol = vtableSymbol,
                initialLibraryPath = libPath,
            )
        }
    }
}

/**
 * Reusable Android [ComponentActivity] host wiring Android lifecycle transitions, viewport resize
 * propagation, Tier-2 hot-reload, and binary `EventRing` dispatch into an [NveSurfaceController].
 *
 * Subclasses may override [provideHostConfig], [provideBridgeBackend], [initialProps], and
 * [onSurfaceEvent] for custom embedding or deterministic Robolectric/JVM testing.
 */
open class NveHostActivity : ComponentActivity() {
    lateinit var hostConfig: NveHostConfig
        private set

    lateinit var surfaceController: NveSurfaceController
        private set

    val surfaceId: Int
        get() = surfaceController.surfaceId

    val surfaceState: NveSurfaceState
        get() = surfaceController.state

    var isSurfaceAttached: Boolean = false
        private set

    var lastLoadedLibraryPath: String? = null
        private set

    protected open fun provideHostConfig(): NveHostConfig = NveHostConfig.fromIntent(intent)

    protected open fun provideBridgeBackend(): NveBridgeBackend = NveNativeBridge

    protected open fun initialProps(): Map<String, Any?> = emptyMap()

    protected open fun onSurfaceEvent(event: NveEvent) {}

    protected open fun installHostContent(controller: NveSurfaceController) {
        val props = initialProps()
        setContent {
            NveSurface(
                componentName = hostConfig.componentName,
                modifier = Modifier.fillMaxSize(),
                vtableSymbol = hostConfig.vtableSymbol,
                backend = hostConfig.backend,
                props = props,
                controller = controller,
                onEvent = ::onSurfaceEvent,
            )
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        hostConfig = provideHostConfig()
        surfaceController = NveSurfaceController(
            componentName = hostConfig.componentName,
            vtableSymbol = hostConfig.vtableSymbol,
            backend = hostConfig.backend,
            surfaceId = hostConfig.surfaceId,
            bridge = provideBridgeBackend(),
        )
        surfaceController.onEventCallback = ::onSurfaceEvent

        attachSurface()
        syncViewportFromResources()
        hostConfig.initialLibraryPath?.let { path ->
            reloadModule(path)
        }
        val props = initialProps()
        if (props.isNotEmpty()) {
            surfaceController.updateProps(props)
        }
        installHostContent(surfaceController)
    }

    override fun onStart() {
        super.onStart()
        if (!isSurfaceAttached && !surfaceController.isDisposed) {
            attachSurface()
            syncViewportFromResources()
            surfaceController.requestRender()
        }
    }

    override fun onResume() {
        super.onResume()
        if (isSurfaceAttached) {
            surfaceController.drainAndForwardEvents()
        }
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        val density = if (newConfig.densityDpi > 0) {
            (newConfig.densityDpi / 160f).coerceAtLeast(1f)
        } else {
            resources.displayMetrics.density.coerceAtLeast(1f)
        }
        val widthDp = if (newConfig.screenWidthDp > 0) {
            newConfig.screenWidthDp.toFloat()
        } else {
            resources.displayMetrics.widthPixels / density
        }
        val heightDp = if (newConfig.screenHeightDp > 0) {
            newConfig.screenHeightDp.toFloat()
        } else {
            resources.displayMetrics.heightPixels / density
        }
        resizeSurface(widthDp, heightDp, density)
    }

    override fun onStop() {
        if (hostConfig.detachOnStop) {
            detachSurface()
        } else if (isSurfaceAttached) {
            surfaceController.flushEvents()
        }
        super.onStop()
    }

    override fun onDestroy() {
        isSurfaceAttached = false
        if (::surfaceController.isInitialized) {
            surfaceController.dispose()
        }
        super.onDestroy()
    }

    /**
     * Attaches the native surface handle while keeping the configured [surfaceId].
     */
    fun attachSurface(): Boolean {
        if (!::surfaceController.isInitialized || surfaceController.isDisposed) return false
        val attached = surfaceController.attach()
        isSurfaceAttached = attached
        return attached
    }

    /**
     * Detaches the active native surface without permanently disposing the controller so a later
     * [onStart] / [attachSurface] restores the surface under the same [surfaceId].
     */
    fun detachSurface() {
        if (!::surfaceController.isInitialized || surfaceController.isDisposed) return
        surfaceController.detach()
        isSurfaceAttached = false
    }

    /**
     * Resizes the hosted surface in logical dp and renders a fresh frame.
     */
    fun resizeSurface(widthDp: Float, heightDp: Float, scaleFactor: Float): Boolean {
        if (!::surfaceController.isInitialized || surfaceController.isDisposed) return false
        val ok = surfaceController.resize(widthDp, heightDp, scaleFactor)
        if (ok) {
            surfaceController.requestRender()
        }
        return ok
    }

    /**
     * Converts pixel viewport changes (e.g. from window insets or layout listeners) into dp
     * and forwards them to [resizeSurface].
     */
    fun onViewportChanged(widthPx: Int, heightPx: Int, density: Float): Boolean {
        val safeDensity = density.coerceAtLeast(1f)
        return resizeSurface(
            widthDp = widthPx.coerceAtLeast(0) / safeDensity,
            heightDp = heightPx.coerceAtLeast(0) / safeDensity,
            scaleFactor = safeDensity,
        )
    }

    /**
     * Performs a Tier-2 hot-swap reload of a compiled Rust app `.so` generation.
     * Leaves the previously running module untouched on failure.
     */
    fun reloadModule(soPath: String): Boolean {
        if (!::surfaceController.isInitialized || !isSurfaceAttached) return false
        val ok = surfaceController.reload(soPath)
        if (ok) {
            lastLoadedLibraryPath = soPath
        }
        return ok
    }

    private fun syncViewportFromResources() {
        val dm = resources.displayMetrics
        val density = dm.density.coerceAtLeast(1f)
        val widthDp = dm.widthPixels.coerceAtLeast(0) / density
        val heightDp = dm.heightPixels.coerceAtLeast(0) / density
        if (widthDp > 0f && heightDp > 0f) {
            surfaceController.resize(widthDp, heightDp, density)
            surfaceController.requestRender()
        }
    }
}
