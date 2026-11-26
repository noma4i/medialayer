use medialayer::prefetch::CacheStrategy;
use medialayer::spec::{type_name, MediaSpec};

use crate::source::{sources, Source};

pub fn kotlin(spec: &MediaSpec, package: &str) -> String {
    let mut out = format!("package {package}\n\nimport com.noma4i.medialayer.Kind\nimport com.noma4i.medialayer.MediaSource\n");
    for source in sources(spec) {
        out.push('\n');
        out.push_str(&declaration(&source));
    }
    out.push_str(&format!("\nobject MediaSpecJson {{\n    const val VALUE: String = \"\"\"{}\"\"\"\n}}\n", spec.to_json()));
    out
}

fn declaration(source: &Source) -> String {
    let optional = source.optional.iter().map(|label| format!("    override val {label}: String? = null,\n")).collect::<String>();
    let size = source
        .size
        .map(|(width, height)| format!("    override val size: Pair<Int, Int>? get() = Pair({width}, {height})\n"))
        .unwrap_or_default();
    let strategy = match source.strategy {
        CacheStrategy::Window => "",
        CacheStrategy::Whole => "    override val strategy: com.noma4i.medialayer.CacheStrategy get() = com.noma4i.medialayer.CacheStrategy.WHOLE\n",
        CacheStrategy::OnDemand => "    override val strategy: com.noma4i.medialayer.CacheStrategy get() = com.noma4i.medialayer.CacheStrategy.ON_DEMAND\n",
    };
    format!(
        "data class {name}(\n    override val url: String,\n{optional}    override val headers: Map<String, String> = emptyMap(),\n) : MediaSource {{\n    override val kind: Kind get() = Kind.{kind}\n{size}{strategy}}}\n",
        name = type_name(source.name),
        kind = source.kind.to_uppercase(),
    )
}

#[cfg(test)]
mod kotlin_tests;
