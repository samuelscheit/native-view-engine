{#- Record and enum emission shared by every C ABI header template. -#}
{%- macro enum_def(e) %}
{%- if let Some(doc) = e.doc %}
/* {{ spell::one_line(doc) }} */
{%- endif %}
typedef enum {{ e.name }} {
{%- for v in e.variants %}
  {{ model::c_enum_prefix(e) }}{{ spell::screaming(v.name) }} = {{ spell::int(v.value) }},
{%- endfor %}
} {{ e.name }};
{% endmacro -%}

{%- macro typedefs(records) %}
/* Every record is declared up front so records can point at each other in any order. */
{%- for r in records %}
typedef {% if r.is_union %}union{% else %}struct{% endif %} {{ r.name }} {{ r.name }};
{%- endfor %}
{%- endmacro -%}

{%- macro definitions(records, aliases) %}
{%- for r in records %}
{%- if let Some(doc) = r.doc %}
/* {{ spell::one_line(doc) }} */
{%- endif %}
{%- if let Some(n) = r.slab_align %}
/* Rust aligns `{{ r.name }}` slabs to {{ n }} bytes (`align({{ n }})`); that changes no size or layout here, so C
 * keeps the natural alignment Swift's ClangImporter imports as a plain value type. A C producer that
 * hands Rust a `{{ r.name }}` array must still allocate it {{ n }}-byte aligned (`aligned_alloc({{ n }}, ...)`). */
{%- endif %}
{% if r.is_union %}union{% else %}struct{% endif %} {{ r.name }} {
{%- for f in r.fields %}
  {% if loop.first %}{% if let Some(n) = r.alignas %}NVE_ALIGNAS({{ n }}) {% endif %}{% endif %}{{ c::field_decl(aliases, f) }};
{%- endfor %}
};
{% endfor %}
{%- endmacro -%}

{#- Layout locks; `NveDetailU64AlignProbe` (declared by `nve_abi.generated.h`) tells i386 System V apart. -#}
{%- macro asserts(records) %}
#define NVE_ABI_U64_FIELD_ALIGN offsetof(NveDetailU64AlignProbe, value)
{%- for r in records %}
NVE_STATIC_ASSERT(sizeof({{ r.name }}) == {{ c::width(r.size) }}, "{{ r.name }} size");
NVE_STATIC_ASSERT(NVE_ALIGNOF({{ r.name }}) == {{ c::width(r.align) }}, "{{ r.name }} alignment");
{%- for f in r.fields %}
NVE_STATIC_ASSERT(offsetof({{ r.name }}, {{ f.name() }}) == {{ c::width(f.offset) }}, "{{ r.name }}.{{ f.name() }} offset");
{%- endfor %}
{%- endfor %}
#undef NVE_ABI_U64_FIELD_ALIGN
{%- endmacro -%}
