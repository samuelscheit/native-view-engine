package com.facebook.react.bridge

interface ReadableArray {
    fun size(): Int
    fun getBoolean(index: Int): Boolean
    fun getInt(index: Int): Int
    fun getDouble(index: Int): Double
    fun getString(index: Int): String?
}

class SimpleReadableArray(private val values: List<Any?>) : ReadableArray {
    override fun size(): Int = values.size
    override fun getBoolean(index: Int): Boolean = values[index] as? Boolean ?: false
    override fun getInt(index: Int): Int = (values[index] as? Number)?.toInt() ?: 0
    override fun getDouble(index: Int): Double = (values[index] as? Number)?.toDouble() ?: 0.0
    override fun getString(index: Int): String? = values[index] as? String
}

interface WritableMap {
    fun putBoolean(key: String, value: Boolean)
    fun putInt(key: String, value: Int)
    fun putDouble(key: String, value: Double)
    fun putString(key: String, value: String?)
    fun toHashMap(): Map<String, Any?>
}

class SimpleWritableMap : WritableMap {
    private val map = LinkedHashMap<String, Any?>()
    override fun putBoolean(key: String, value: Boolean) { map[key] = value }
    override fun putInt(key: String, value: Int) { map[key] = value }
    override fun putDouble(key: String, value: Double) { map[key] = value }
    override fun putString(key: String, value: String?) { map[key] = value }
    override fun toHashMap(): Map<String, Any?> = LinkedHashMap(map)
}

object Arguments {
    fun createMap(): WritableMap = SimpleWritableMap()
}
