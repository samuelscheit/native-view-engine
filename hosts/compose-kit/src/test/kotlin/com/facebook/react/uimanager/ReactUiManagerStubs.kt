package com.facebook.react.uimanager

import android.content.Context
import android.content.ContextWrapper
import android.view.View
import com.facebook.react.bridge.ReadableArray
import com.facebook.react.bridge.WritableMap
import com.facebook.react.uimanager.events.RCTEventEmitter

data class RecordedRnEvent(val targetTag: Int, val eventName: String, val payload: Map<String, Any?>)

class ThemedReactContext(base: Context) : ContextWrapper(base) {
    val recordedEvents = ArrayList<RecordedRnEvent>()

    private val emitter = object : RCTEventEmitter {
        override fun receiveEvent(targetTag: Int, eventName: String, event: WritableMap?) {
            recordedEvents.add(RecordedRnEvent(targetTag, eventName, event?.toHashMap() ?: emptyMap()))
        }
    }

    @Suppress("UNCHECKED_CAST")
    fun <T> getJSModule(jsInterface: Class<T>): T {
        if (jsInterface == RCTEventEmitter::class.java) return emitter as T
        throw IllegalArgumentException("Unsupported JS module stub: $jsInterface")
    }
}

abstract class SimpleViewManager<T : View> {
    abstract fun getName(): String
    abstract fun createViewInstance(reactContext: ThemedReactContext): T
    open fun onAfterUpdateTransaction(view: T) {}
    open fun onDropViewInstance(view: T) {}
    open fun receiveCommand(root: T, commandId: String, args: ReadableArray?) {}
    open fun getExportedCustomDirectEventTypeConstants(): MutableMap<String, Any>? = null
    open fun getExportedCustomBubblingEventTypeConstants(): MutableMap<String, Any>? = null
}
