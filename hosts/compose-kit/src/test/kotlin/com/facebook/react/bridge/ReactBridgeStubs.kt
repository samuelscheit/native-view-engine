package com.facebook.react.bridge

enum class ReadableType {
    Null,
    Boolean,
    Number,
    String,
    Map,
    Array,
}

interface ReadableMapKeySetIterator {
    fun hasNextKey(): Boolean
    fun nextKey(): String
}

interface ReadableArray {
    fun size(): Int
    fun isNull(index: Int): Boolean
    fun getBoolean(index: Int): Boolean
    fun getInt(index: Int): Int
    fun getDouble(index: Int): Double
    fun getString(index: Int): String?
    fun getArray(index: Int): ReadableArray?
    fun getMap(index: Int): ReadableMap?
    fun getType(index: Int): ReadableType
}

interface ReadableMap {
    fun hasKey(name: String): Boolean
    fun isNull(name: String): Boolean
    fun getBoolean(name: String): Boolean
    fun getInt(name: String): Int
    fun getDouble(name: String): Double
    fun getString(name: String): String?
    fun getArray(name: String): ReadableArray?
    fun getMap(name: String): ReadableMap?
    fun getType(name: String): ReadableType
    fun keySetIterator(): ReadableMapKeySetIterator
}

interface WritableArray : ReadableArray {
    fun pushNull()
    fun pushBoolean(value: Boolean)
    fun pushInt(value: Int)
    fun pushDouble(value: Double)
    fun pushString(value: String?)
    fun pushArray(array: ReadableArray?)
    fun pushMap(map: ReadableMap?)
    fun toArrayList(): List<Any?>
}

interface WritableMap : ReadableMap {
    fun putNull(key: String)
    fun putBoolean(key: String, value: Boolean)
    fun putInt(key: String, value: Int)
    fun putDouble(key: String, value: Double)
    fun putString(key: String, value: String?)
    fun putArray(key: String, value: ReadableArray?)
    fun putMap(key: String, value: ReadableMap?)
    fun toHashMap(): Map<String, Any?>
}

private fun classifyValue(v: Any?): ReadableType = when (v) {
    null -> ReadableType.Null
    is Boolean -> ReadableType.Boolean
    is Number -> ReadableType.Number
    is String -> ReadableType.String
    is ReadableMap -> ReadableType.Map
    is ReadableArray -> ReadableType.Array
    else -> ReadableType.String
}

open class SimpleReadableArray(initial: List<Any?> = emptyList()) : WritableArray {
    private val values = ArrayList<Any?>(initial)

    /** Mutates this array in place (tests proving callers do not retain stale bridge refs). */
    fun mutateClear() {
        values.clear()
    }

    fun mutateSet(index: Int, value: Any?) {
        values[index] = value
    }

    override fun size(): Int = values.size
    override fun isNull(index: Int): Boolean = values[index] == null
    override fun getBoolean(index: Int): Boolean = values[index] as? Boolean ?: false
    override fun getInt(index: Int): Int = (values[index] as? Number)?.toInt() ?: 0
    override fun getDouble(index: Int): Double = (values[index] as? Number)?.toDouble() ?: 0.0
    override fun getString(index: Int): String? = values[index] as? String
    override fun getArray(index: Int): ReadableArray? = values[index] as? ReadableArray
    override fun getMap(index: Int): ReadableMap? = values[index] as? ReadableMap
    override fun getType(index: Int): ReadableType = classifyValue(values[index])

    override fun pushNull() { values.add(null) }
    override fun pushBoolean(value: Boolean) { values.add(value) }
    override fun pushInt(value: Int) { values.add(value) }
    override fun pushDouble(value: Double) { values.add(value) }
    override fun pushString(value: String?) { values.add(value) }
    override fun pushArray(array: ReadableArray?) { values.add(array) }
    override fun pushMap(map: ReadableMap?) { values.add(map) }

    override fun toArrayList(): List<Any?> = values.map { elem ->
        when (elem) {
            is WritableMap -> elem.toHashMap()
            is WritableArray -> elem.toArrayList()
            else -> elem
        }
    }
}

open class SimpleReadableMap(initial: Map<String, Any?> = emptyMap()) : WritableMap {
    private val map = LinkedHashMap<String, Any?>(initial)

    /** Mutates this map in place (tests proving callers do not retain stale bridge refs). */
    fun mutateClear() {
        map.clear()
    }

    override fun hasKey(name: String): Boolean = map.containsKey(name)
    override fun isNull(name: String): Boolean = map[name] == null
    override fun getBoolean(name: String): Boolean = map[name] as? Boolean ?: false
    override fun getInt(name: String): Int = (map[name] as? Number)?.toInt() ?: 0
    override fun getDouble(name: String): Double = (map[name] as? Number)?.toDouble() ?: 0.0
    override fun getString(name: String): String? = map[name] as? String
    override fun getArray(name: String): ReadableArray? = map[name] as? ReadableArray
    override fun getMap(name: String): ReadableMap? = map[name] as? ReadableMap
    override fun getType(name: String): ReadableType = classifyValue(map[name])
    override fun keySetIterator(): ReadableMapKeySetIterator {
        val it = ArrayList(map.keys).iterator()
        return object : ReadableMapKeySetIterator {
            override fun hasNextKey(): Boolean = it.hasNext()
            override fun nextKey(): String = it.next()
        }
    }

    override fun putNull(key: String) { map[key] = null }
    override fun putBoolean(key: String, value: Boolean) { map[key] = value }
    override fun putInt(key: String, value: Int) { map[key] = value }
    override fun putDouble(key: String, value: Double) { map[key] = value }
    override fun putString(key: String, value: String?) { map[key] = value }
    override fun putArray(key: String, value: ReadableArray?) { map[key] = value }
    override fun putMap(key: String, value: ReadableMap?) { map[key] = value }

    override fun toHashMap(): Map<String, Any?> {
        val out = LinkedHashMap<String, Any?>(map.size)
        for ((k, v) in map) {
            out[k] = when (v) {
                is WritableMap -> v.toHashMap()
                is WritableArray -> v.toArrayList()
                else -> v
            }
        }
        return out
    }
}

class SimpleWritableMap : SimpleReadableMap()
class SimpleWritableArray : SimpleReadableArray()

object Arguments {
    fun createMap(): WritableMap = SimpleWritableMap()
    fun createArray(): WritableArray = SimpleWritableArray()
}
