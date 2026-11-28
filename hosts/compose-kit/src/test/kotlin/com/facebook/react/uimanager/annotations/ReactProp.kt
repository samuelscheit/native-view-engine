package com.facebook.react.uimanager.annotations

@Retention(AnnotationRetention.RUNTIME)
@Target(AnnotationTarget.FUNCTION)
annotation class ReactProp(
    val name: String,
    val customType: String = "__default_type__",
    val defaultBoolean: Boolean = false,
    val defaultInt: Int = 0,
    val defaultFloat: Float = 0f,
    val defaultDouble: Double = 0.0,
)
