use medialayer::spec::{type_name, MediaSpec};

use crate::source::{sources, Source};

pub fn swift(spec: &MediaSpec) -> String {
    let mut out = String::from("import MediaLayer\n");
    for source in sources(spec) {
        out.push('\n');
        out.push_str(&declaration(&source));
    }
    out.push_str(&format!("\npublic enum MediaSpecJSON {{\n    public static let value = #\"\"\"\n{}\n\"\"\"#\n}}\n", spec.to_json()));
    out
}

fn declaration(source: &Source) -> String {
    let optional = source.optional;
    let field = optional.map(|label| format!("    public let {label}: String?\n")).unwrap_or_default();
    let parameter = optional.map(|label| format!("{label}: String? = nil, ")).unwrap_or_default();
    let assignment = optional.map(|label| format!("        self.{label} = {label}\n")).unwrap_or_default();
    let size = source
        .size
        .map(|(width, height)| format!("    public var size: MediaSize? {{ MediaSize(width: {width}, height: {height}) }}\n"))
        .unwrap_or_default();
    format!(
        "public struct {name}: MediaSource, Sendable {{\n    public let url: String\n{field}    public let headers: [String: String]\n    public var kind: Kind {{ .{kind} }}\n{size}\n    public init(url: String, {parameter}headers: [String: String] = [:]) {{\n        self.url = url\n{assignment}        self.headers = headers\n    }}\n}}\n",
        name = type_name(source.name),
        kind = source.kind,
    )
}

#[cfg(test)]
mod swift_tests;
