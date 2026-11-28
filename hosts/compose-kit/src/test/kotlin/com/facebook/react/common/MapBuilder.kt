package com.facebook.react.common

object MapBuilder {
    class Builder<K, V> {
        private val map = LinkedHashMap<K, V>()
        fun put(k: K, v: V): Builder<K, V> {
            map[k] = v
            return this
        }
        fun build(): MutableMap<K, V> = map
    }

    fun <K, V> builder(): Builder<K, V> = Builder()

    fun <K, V> of(k1: K, v1: V): MutableMap<K, V> {
        val m = LinkedHashMap<K, V>()
        m[k1] = v1
        return m
    }
}
