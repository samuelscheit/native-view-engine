package com.facebook.react.uimanager.events

import com.facebook.react.bridge.WritableMap

interface RCTEventEmitter {
    fun receiveEvent(targetTag: Int, eventName: String, event: WritableMap?)
}
