use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use heck::{ToSnakeCase, ToUpperCamelCase};
use serde::Deserialize;

const CORE_TYPES: &[&str] = &["InputFile"];

const CORE_METHODS: &[&str] = &[];

// These shortcuts have custom implementations in `src/bot.rs` because they
// expose additional aiogram-style convenience behavior.
const CORE_BOT_SHORTCUTS: &[&str] = &[
    "getMe",
    "getUpdates",
    "getFile",
    "sendMessage",
    "answerCallbackQuery",
];

#[derive(Debug, Deserialize)]
struct EntityFile {
    object: ApiObject,
}

#[derive(Debug, Deserialize)]
struct ApiObject {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    annotations: Vec<Annotation>,
}

#[derive(Debug, Deserialize)]
struct Annotation {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    description: String,
}

#[derive(Debug)]
struct UnionDef {
    name: String,
    discriminator: Option<String>,
    variants: Vec<UnionVariant>,
}

#[derive(Debug)]
struct UnionVariant {
    name: String,
    tag: Option<String>,
}

#[derive(Debug)]
struct EnumDef {
    name: String,
    kind: EnumKind,
    variants: Vec<EnumVariant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnumKind {
    String,
    Integer,
}

#[derive(Debug)]
struct EnumVariant {
    name: String,
    value: String,
}

#[derive(Debug, Clone)]
struct PythonField {
    annotation: String,
    default: Option<String>,
}

type PythonFields = BTreeMap<(String, String), PythonField>;

#[derive(Debug, Deserialize)]
struct BoundAlias {
    method: String,
    #[serde(default)]
    fill: BTreeMap<String, String>,
    #[serde(default)]
    ignore: Vec<String>,
    #[serde(default)]
    code: Option<String>,
}

type BoundAliases = BTreeMap<String, BTreeMap<String, BoundAlias>>;

#[derive(Clone, Copy)]
enum Destination {
    Types,
    Methods,
    Return,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("xtask failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_default();
    if command != "generate" {
        return Err("usage: cargo run -p xtask -- generate [--upstream PATH]".into());
    }

    let mut upstream = PathBuf::from("aiogram");
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--upstream" => upstream = PathBuf::from(args.next().ok_or("--upstream needs a path")?),
            unknown => return Err(format!("unknown argument: {unknown}").into()),
        }
    }

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask must be inside the workspace")
        .to_owned();
    let upstream = if upstream.is_absolute() {
        upstream
    } else {
        root.join(upstream)
    };
    let types = read_entities(&upstream.join(".butcher/types"))?;
    let methods = read_entities(&upstream.join(".butcher/methods"))?;
    let type_names: BTreeSet<String> = types.iter().map(|object| object.name.clone()).collect();
    let unions = read_unions(&upstream.join("aiogram/types"), &type_names, &types)?;
    let enums = read_enums(&upstream.join("aiogram/enums"))?;
    let type_fields = read_python_fields(&upstream.join("aiogram/types"), &types)?;
    let method_fields = read_python_fields(&upstream.join("aiogram/methods"), &methods)?;
    let bound_aliases = read_bound_aliases(&upstream.join(".butcher/types"), &types)?;

    let generated_types = generate_types(&types, &type_names, &unions, &type_fields, &enums);
    let generated_enums = generate_enums(&enums);
    let generated_methods =
        generate_methods(&methods, &type_names, &upstream, &method_fields, &enums)?;
    let generated_bot =
        generate_bot_shortcuts(&methods, &type_names, &upstream, &method_fields, &enums)?;
    let generated_bound = generate_bound_shortcuts(
        &bound_aliases,
        &methods,
        &type_names,
        &method_fields,
        &enums,
    )?;
    let types_path = root.join("src/types/generated.rs");
    let enums_path = root.join("src/enums/generated.rs");
    let methods_path = root.join("src/methods/generated.rs");
    let bot_path = root.join("src/bot/generated.rs");
    let bound_path = root.join("src/types/bound.rs");
    fs::create_dir_all(types_path.parent().unwrap())?;
    fs::create_dir_all(enums_path.parent().unwrap())?;
    fs::create_dir_all(methods_path.parent().unwrap())?;
    fs::create_dir_all(bot_path.parent().unwrap())?;
    fs::create_dir_all(bound_path.parent().unwrap())?;
    fs::write(&types_path, generated_types)?;
    fs::write(&enums_path, generated_enums)?;
    fs::write(&methods_path, generated_methods)?;
    fs::write(&bot_path, generated_bot)?;
    fs::write(&bound_path, generated_bound)?;

    println!(
        "generated {} API types (Python annotations {}/{}), {} enums, {} API methods (Python annotations {}/{}), and {} bound object methods from {}",
        types.len(),
        type_fields.len(),
        types
            .iter()
            .map(|object| object.annotations.len())
            .sum::<usize>(),
        enums.len(),
        methods.len(),
        method_fields.len(),
        methods
            .iter()
            .map(|object| object.annotations.len())
            .sum::<usize>(),
        bound_aliases.values().map(BTreeMap::len).sum::<usize>(),
        upstream.display()
    );
    Ok(())
}

fn read_bound_aliases(
    root: &Path,
    objects: &[ApiObject],
) -> Result<BoundAliases, Box<dyn std::error::Error>> {
    let mut aliases = BTreeMap::new();
    for object in objects {
        let path = root.join(&object.name).join("aliases.yml");
        if !path.exists() {
            continue;
        }
        let source = fs::read_to_string(&path)?;
        let mut value: serde_yaml::Value = serde_yaml::from_str(&source)?;
        value.apply_merge()?;
        let parsed: BTreeMap<String, BoundAlias> = serde_yaml::from_value(value)?;
        aliases.insert(object.name.clone(), parsed);
    }
    Ok(aliases)
}

fn read_python_fields(
    root: &Path,
    objects: &[ApiObject],
) -> Result<PythonFields, Box<dyn std::error::Error>> {
    let mut fields = BTreeMap::new();
    for object in objects {
        let path = root.join(format!("{}.py", object.name.to_snake_case()));
        let source = fs::read_to_string(&path)?;
        let lines: Vec<_> = source.lines().collect();
        for field in &object.annotations {
            let rust_name = field_name(&field.name).0;
            let candidates = [field.name.as_str(), rust_name.as_str()];
            let parsed = candidates.iter().find_map(|name| {
                let prefix = format!("    {name}: ");
                lines.iter().enumerate().find_map(|(index, line)| {
                    let value = line.strip_prefix(&prefix)?;
                    let (annotation, default) = match value.split_once(" = ") {
                        Some((annotation, default)) => {
                            let mut default = default.trim().to_owned();
                            let mut balance = delimiter_balance(&default);
                            for continuation in &lines[index + 1..] {
                                if balance <= 0 {
                                    break;
                                }
                                default.push(' ');
                                default.push_str(continuation.trim());
                                balance += delimiter_balance(continuation);
                            }
                            (annotation.trim(), Some(default))
                        }
                        None => (value.trim(), None),
                    };
                    Some(PythonField {
                        annotation: annotation.to_owned(),
                        default,
                    })
                })
            });
            if let Some(parsed) = parsed {
                fields.insert((object.name.clone(), field.name.clone()), parsed);
            }
        }
    }
    Ok(fields)
}

fn delimiter_balance(value: &str) -> i64 {
    value
        .chars()
        .map(|character| match character {
            '(' | '[' | '{' => 1,
            ')' | ']' | '}' => -1,
            _ => 0,
        })
        .sum()
}

fn default_reference(field: &PythonField) -> Option<&str> {
    let default = field.default.as_deref()?;
    let (_, value) = default.split_once("Default(\"")?;
    value.split_once('"').map(|(name, _)| name)
}

fn read_enums(root: &Path) -> Result<Vec<EnumDef>, Box<dyn std::error::Error>> {
    let mut paths: Vec<PathBuf> = fs::read_dir(root)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension().and_then(|extension| extension.to_str()) == Some("py")
                && path.file_name().and_then(|name| name.to_str()) != Some("__init__.py")
        })
        .collect();
    paths.sort();

    let mut definitions = Vec::with_capacity(paths.len());
    for path in paths {
        let source = fs::read_to_string(&path)?;
        let class = source
            .lines()
            .find(|line| line.starts_with("class "))
            .ok_or_else(|| format!("enum class not found in {}", path.display()))?;
        let declaration = class
            .strip_prefix("class ")
            .and_then(|line| line.strip_suffix(':'))
            .ok_or_else(|| format!("invalid enum class in {}", path.display()))?;
        let (name, bases) = declaration
            .split_once('(')
            .ok_or_else(|| format!("enum bases not found in {}", path.display()))?;
        let kind = if bases.starts_with("str,") {
            EnumKind::String
        } else if bases.starts_with("int,") {
            EnumKind::Integer
        } else {
            return Err(format!("unsupported enum base in {}: {bases}", path.display()).into());
        };

        let variants = source
            .lines()
            .filter_map(|line| line.strip_prefix("    "))
            .filter_map(|line| line.split_once(" = "))
            .filter(|(name, _)| {
                !name.is_empty()
                    && name.chars().all(|character| {
                        character.is_ascii_uppercase()
                            || character.is_ascii_digit()
                            || character == '_'
                    })
            })
            .map(|(name, value)| {
                let value = match kind {
                    EnumKind::String => serde_json::from_str::<String>(value),
                    EnumKind::Integer => Ok(value.to_owned()),
                }
                .map_err(|error| format!("invalid enum value in {}: {error}", path.display()))?;
                Ok(EnumVariant {
                    name: name.to_upper_camel_case(),
                    value,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        if variants.is_empty() {
            return Err(format!("enum variants not found in {}", path.display()).into());
        }
        definitions.push(EnumDef {
            name: name.to_owned(),
            kind,
            variants,
        });
    }
    Ok(definitions)
}

fn read_entities(root: &Path) -> Result<Vec<ApiObject>, Box<dyn std::error::Error>> {
    let mut paths: Vec<PathBuf> = fs::read_dir(root)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path().join("entity.json")))
        .filter(|path| path.is_file())
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let entity: EntityFile = serde_json::from_slice(&fs::read(&path)?)?;
            Ok(entity.object)
        })
        .collect()
}

fn read_unions(
    root: &Path,
    type_names: &BTreeSet<String>,
    objects: &[ApiObject],
) -> Result<Vec<UnionDef>, Box<dyn std::error::Error>> {
    let objects: BTreeMap<_, _> = objects
        .iter()
        .map(|object| (object.name.as_str(), object))
        .collect();
    let mut paths: Vec<PathBuf> = fs::read_dir(root)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with("_union.py"))
        })
        .collect();
    paths.sort();
    let mut unions = Vec::new();
    for path in paths {
        let stem = path.file_stem().and_then(|value| value.to_str()).unwrap();
        let name = stem.to_upper_camel_case();
        let source = fs::read_to_string(path)?;
        let mut variant_names = Vec::new();
        for token in
            source.split(|character: char| !(character.is_alphanumeric() || character == '_'))
        {
            if type_names.contains(token) && !variant_names.iter().any(|variant| variant == token) {
                variant_names.push(token.to_owned());
            }
        }
        let explicit = source
            .split_once("Field(discriminator=\"")
            .and_then(|(_, tail)| tail.split_once('"'))
            .map(|(value, _)| value.to_owned());
        let candidates: Vec<&str> = explicit
            .as_deref()
            .into_iter()
            .chain(["type", "status", "source"])
            .collect();
        let discriminator = if matches!(name.as_str(), "RichTextUnion" | "InputPollOptionUnion") {
            None
        } else {
            candidates.into_iter().find_map(|candidate| {
                let tags: Vec<_> = variant_names
                    .iter()
                    .map(|variant| {
                        objects
                            .get(variant.as_str())
                            .and_then(|object| constant_field(object, candidate))
                    })
                    .collect();
                let unique: BTreeSet<_> = tags.iter().flatten().collect();
                (tags.iter().all(Option::is_some) && unique.len() == tags.len())
                    .then(|| candidate.to_owned())
            })
        };
        let variants = variant_names
            .into_iter()
            .map(|variant| UnionVariant {
                tag: discriminator.as_deref().and_then(|field| {
                    objects
                        .get(variant.as_str())
                        .and_then(|object| constant_field(object, field))
                }),
                name: variant,
            })
            .collect();
        unions.push(UnionDef {
            name,
            discriminator,
            variants,
        });
    }
    Ok(unions)
}

fn constant_field(object: &ApiObject, field: &str) -> Option<String> {
    let description = &object
        .annotations
        .iter()
        .find(|annotation| annotation.name == field)?
        .description;
    for quote in ['\'', '"'] {
        let marker = format!("always {quote}");
        if let Some((_, tail)) = description.split_once(&marker)
            && let Some((value, _)) = tail.split_once(quote)
        {
            return Some(value.to_owned());
        }
    }
    None
}

fn generate_enums(definitions: &[EnumDef]) -> String {
    let mut output = generated_header("aiogram-compatible enums");
    output.push_str(
        "use std::fmt;\nuse std::str::FromStr;\n\nuse serde::{Deserialize, Serialize};\n\n",
    );
    output.push_str(&format!(
        "/// Number of enum definitions in the pinned upstream snapshot.\npub const API_ENUM_COUNT: usize = {};\n\n",
        definitions.len()
    ));
    output.push_str(
        "/// Error returned when a string is not a member of a Telegram enum.\n#[derive(Debug, Clone, PartialEq, Eq)]\npub struct ParseTelegramEnumError {\n    pub enum_name: &'static str,\n    pub value: String,\n}\n\nimpl fmt::Display for ParseTelegramEnumError {\n    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {\n        write!(formatter, \"unknown {} value: {}\", self.enum_name, self.value)\n    }\n}\n\nimpl std::error::Error for ParseTelegramEnumError {}\n\n",
    );

    for definition in definitions {
        match definition.kind {
            EnumKind::String => push_string_enum(&mut output, definition),
            EnumKind::Integer => push_integer_enum(&mut output, definition),
        }
    }
    output
}

fn push_string_enum(output: &mut String, definition: &EnumDef) {
    output.push_str("#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]\n");
    output.push_str(&format!("pub enum {} {{\n", definition.name));
    for variant in &definition.variants {
        output.push_str(&format!(
            "    #[serde(rename = {:?})]\n    {},\n",
            variant.value, variant.name
        ));
    }
    output.push_str("}\n\n");
    output.push_str(&format!(
        "impl {} {{\n    pub const fn as_str(self) -> &'static str {{\n        match self {{\n",
        definition.name
    ));
    for variant in &definition.variants {
        output.push_str(&format!(
            "            Self::{} => {:?},\n",
            variant.name, variant.value
        ));
    }
    output.push_str("        }\n    }\n}\n\n");
    output.push_str(&format!(
        "impl fmt::Display for {} {{\n    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {{\n        formatter.write_str(self.as_str())\n    }}\n}}\n\n",
        definition.name
    ));
    output.push_str(&format!(
        "impl AsRef<str> for {} {{\n    fn as_ref(&self) -> &str {{ self.as_str() }}\n}}\n\n",
        definition.name
    ));
    output.push_str(&format!(
        "impl From<{}> for String {{\n    fn from(value: {}) -> Self {{ value.as_str().to_owned() }}\n}}\n\n",
        definition.name, definition.name
    ));
    output.push_str(&format!(
        "impl FromStr for {} {{\n    type Err = ParseTelegramEnumError;\n\n    fn from_str(value: &str) -> Result<Self, Self::Err> {{\n        match value {{\n",
        definition.name
    ));
    for variant in &definition.variants {
        output.push_str(&format!(
            "            {:?} => Ok(Self::{}),\n",
            variant.value, variant.name
        ));
    }
    output.push_str(&format!(
        "            _ => Err(ParseTelegramEnumError {{ enum_name: {:?}, value: value.to_owned() }}),\n        }}\n    }}\n}}\n\n",
        definition.name
    ));
}

fn push_integer_enum(output: &mut String, definition: &EnumDef) {
    output.push_str("#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]\n#[repr(u32)]\n");
    output.push_str(&format!("pub enum {} {{\n", definition.name));
    for variant in &definition.variants {
        output.push_str(&format!("    {} = {},\n", variant.name, variant.value));
    }
    output.push_str("}\n\n");
    output.push_str(&format!(
        "impl {} {{\n    pub const fn as_u32(self) -> u32 {{ self as u32 }}\n\n    pub const fn from_u32(value: u32) -> Option<Self> {{\n        match value {{\n",
        definition.name
    ));
    for variant in &definition.variants {
        output.push_str(&format!(
            "            value if value == Self::{} as u32 => Some(Self::{}),\n",
            variant.name, variant.name
        ));
    }
    output.push_str("            _ => None,\n        }\n    }\n}\n\n");
    output.push_str(&format!(
        "impl Serialize for {} {{\n    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>\n    where\n        S: serde::Serializer,\n    {{\n        serializer.serialize_u32(self.as_u32())\n    }}\n}}\n\n",
        definition.name
    ));
    output.push_str(&format!(
        "impl<'de> Deserialize<'de> for {} {{\n    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>\n    where\n        D: serde::Deserializer<'de>,\n    {{\n        let value = u32::deserialize(deserializer)?;\n        Self::from_u32(value).ok_or_else(|| serde::de::Error::custom(format!(\"unknown {} value: {{value}}\")))\n    }}\n}}\n\n",
        definition.name, definition.name
    ));
    output.push_str(&format!(
        "impl From<{}> for u32 {{\n    fn from(value: {}) -> Self {{ value.as_u32() }}\n}}\n\n",
        definition.name, definition.name
    ));
}

fn generate_types(
    objects: &[ApiObject],
    type_names: &BTreeSet<String>,
    unions: &[UnionDef],
    python_fields: &PythonFields,
    enums: &[EnumDef],
) -> String {
    let mut output = generated_header("Telegram Bot API types");
    output.push_str(
        "use std::collections::BTreeMap;\n\nuse serde::{Deserialize, Serialize};\nuse serde_json::Value;\n\nuse super::{CollectFiles, InputFileUpload};\n\n",
    );
    output.push_str(&format!(
        "/// Number of entity definitions in the pinned upstream snapshot.\npub const API_ENTITY_COUNT: usize = {};\n\n",
        objects.len()
    ));
    output.push_str(&format!(
        "/// Number of final Python field annotations mapped into generated Rust types.\npub const MAPPED_PYTHON_TYPE_ANNOTATION_COUNT: usize = {};\n\n",
        python_fields.len()
    ));

    for object in objects {
        if CORE_TYPES.contains(&object.name.as_str()) {
            continue;
        }
        push_doc(&mut output, &object.description);
        output.push_str("#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]\n");
        output.push_str(&format!("pub struct {} {{\n", object.name));
        for annotation in &object.annotations {
            push_field(
                &mut output,
                object,
                annotation,
                type_names,
                Destination::Types,
                python_fields,
            );
        }
        output.push_str(
            "    #[serde(flatten, default)]\n    pub extra: BTreeMap<String, Value>,\n}\n\n",
        );
        push_collect_files_impl(&mut output, object, Destination::Types);
        push_constructor(
            &mut output,
            object,
            type_names,
            Destination::Types,
            python_fields,
            enums,
        );
    }
    push_unions(&mut output, unions);
    output
}

fn push_unions(output: &mut String, unions: &[UnionDef]) {
    output.push_str(&format!(
        "/// Number of aiogram-compatible helper unions.\npub const API_UNION_COUNT: usize = {};\n\n",
        unions.len()
    ));
    for union in unions {
        match union.name.as_str() {
            "ChatIdUnion" => {
                output.push_str("pub type ChatIdUnion = super::ChatId;\n\n");
                continue;
            }
            "InputFileUnion" => {
                output.push_str("pub type InputFileUnion = super::InputFile;\n\n");
                continue;
            }
            "DateTimeUnion" => {
                output.push_str("pub type DateTimeUnion = i64;\n\n");
                continue;
            }
            _ => {}
        }
        if union.discriminator.is_some() {
            output.push_str("#[derive(Debug, Clone, PartialEq)]\n");
        } else {
            output.push_str(
                "#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]\n#[serde(untagged)]\n",
            );
        }
        output.push_str(&format!("pub enum {} {{\n", union.name));
        if union.name == "RichTextUnion" {
            output.push_str("    Text(String),\n    List(Vec<RichTextUnion>),\n");
        } else if union.name == "InputPollOptionUnion" {
            output.push_str("    Text(String),\n");
        }
        for variant in &union.variants {
            if variant.name == "InputFile" {
                continue;
            }
            output.push_str(&format!("    {}(Box<{}>),\n", variant.name, variant.name));
        }
        output.push_str("}\n\n");

        if let Some(discriminator) = &union.discriminator {
            output.push_str(&format!(
                "impl Serialize for {} {{\n    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error> where S: serde::Serializer {{\n        match self {{\n",
                union.name
            ));
            for variant in &union.variants {
                if variant.name != "InputFile" {
                    output.push_str(&format!(
                        "            Self::{}(value) => value.serialize(serializer),\n",
                        variant.name
                    ));
                }
            }
            output.push_str("        }\n    }\n}\n\n");
            output.push_str(&format!(
                "impl<'de> Deserialize<'de> for {} {{\n    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error> where D: serde::Deserializer<'de> {{\n        let value = Value::deserialize(deserializer)?;\n        let tag = value.get({:?}).and_then(Value::as_str).ok_or_else(|| serde::de::Error::custom(\"missing union discriminator\"))?;\n        match tag {{\n",
                union.name, discriminator
            ));
            for variant in &union.variants {
                if variant.name == "InputFile" {
                    continue;
                }
                if let Some(tag) = &variant.tag {
                    output.push_str(&format!(
                        "            {:?} => serde_json::from_value::<{}>(value).map(|value| Self::{}(Box::new(value))).map_err(serde::de::Error::custom),\n",
                        tag, variant.name, variant.name
                    ));
                }
            }
            output.push_str(
                "            _ => Err(serde::de::Error::custom(format!(\"unknown union discriminator: {tag}\"))),\n        }\n    }\n}\n\n",
            );
        }

        output.push_str(&format!(
            "impl CollectFiles for {} {{\n    fn collect_files(&self, output: &mut Vec<InputFileUpload>) {{\n        match self {{\n",
            union.name
        ));
        if union.name == "RichTextUnion" {
            output.push_str(
                "            Self::Text(value) => value.collect_files(output),\n            Self::List(value) => value.collect_files(output),\n",
            );
        } else if union.name == "InputPollOptionUnion" {
            output.push_str("            Self::Text(value) => value.collect_files(output),\n");
        }
        for variant in &union.variants {
            if variant.name != "InputFile" {
                output.push_str(&format!(
                    "            Self::{}(value) => value.collect_files(output),\n",
                    variant.name
                ));
            }
        }
        output.push_str("        }\n    }\n}\n\n");

        for variant in &union.variants {
            if variant.name == "InputFile" {
                continue;
            }
            output.push_str(&format!(
                "impl From<{}> for {} {{\n    fn from(value: {}) -> Self {{ Self::{}(Box::new(value)) }}\n}}\n\n",
                variant.name, union.name, variant.name, variant.name
            ));
        }
    }
}

fn generate_methods(
    objects: &[ApiObject],
    type_names: &BTreeSet<String>,
    upstream: &Path,
    python_fields: &PythonFields,
    enums: &[EnumDef],
) -> Result<String, Box<dyn std::error::Error>> {
    let mut output = generated_header("Telegram Bot API methods");
    output.push_str(
        "use std::collections::BTreeMap;\n\nuse serde::Serialize;\nuse serde_json::Value;\n\nuse super::TelegramMethod;\nuse crate::types::{CollectFiles, InputFileUpload};\n\n",
    );
    output.push_str(&format!(
        "/// Number of method definitions in the pinned upstream snapshot.\npub const API_METHOD_COUNT: usize = {};\n\n",
        objects.len()
    ));
    output.push_str(&format!(
        "/// Number of final Python field annotations mapped into generated Rust methods.\npub const MAPPED_PYTHON_METHOD_ANNOTATION_COUNT: usize = {};\n\n",
        python_fields.len()
    ));
    output.push_str(&format!(
        "/// Number of aiogram `Default(...)` field mappings preserved in Rust.\npub const MAPPED_PYTHON_METHOD_DEFAULT_COUNT: usize = {};\n\n",
        python_fields
            .values()
            .filter(|field| default_reference(field).is_some())
            .count()
    ));

    for object in objects {
        if CORE_METHODS.contains(&object.name.as_str()) {
            continue;
        }
        let rust_name = object.name.to_upper_camel_case();
        push_doc(&mut output, &object.description);
        output.push_str("#[derive(Debug, Clone, Serialize)]\n");
        output.push_str(&format!("pub struct {rust_name} {{\n"));
        for annotation in &object.annotations {
            push_field(
                &mut output,
                object,
                annotation,
                type_names,
                Destination::Methods,
                python_fields,
            );
        }
        output.push_str(
            "    #[serde(flatten, default)]\n    pub extra: BTreeMap<String, Value>,\n}\n\n",
        );
        push_collect_files_impl(&mut output, object, Destination::Methods);
        push_constructor(
            &mut output,
            object,
            type_names,
            Destination::Methods,
            python_fields,
            enums,
        );

        let python_path = upstream
            .join("aiogram/methods")
            .join(format!("{}.py", object.name.to_snake_case()));
        let returning = extract_return_type(&python_path)?;
        let returning = python_return_type(&returning, type_names);
        let defaults: Vec<_> = object
            .annotations
            .iter()
            .filter_map(|field| {
                let python = python_fields.get(&(object.name.clone(), field.name.clone()))?;
                Some((field.name.as_str(), default_reference(python)?))
            })
            .collect();
        output.push_str(&format!(
            "impl TelegramMethod for {rust_name} {{\n    type Response = {returning};\n    const NAME: &'static str = {:?};\n    const FIELDS: &'static [&'static str] = &{:?};\n    const DEFAULT_PROPERTIES: &'static [(&'static str, &'static str)] = &{:?};\n}}\n\n",
            object.name,
            object.annotations.iter().map(|field| field.name.as_str()).collect::<Vec<_>>(),
            defaults,
        ));
    }
    Ok(output)
}

fn generate_bot_shortcuts(
    objects: &[ApiObject],
    type_names: &BTreeSet<String>,
    upstream: &Path,
    python_fields: &PythonFields,
    enums: &[EnumDef],
) -> Result<String, Box<dyn std::error::Error>> {
    let mut output = generated_header("convenience methods on `Bot`");
    output.push_str("use super::Bot;\nuse crate::error::Result;\n\n");
    output.push_str(&format!(
        "/// Generated Bot API convenience methods, excluding custom core shortcuts.\npub const GENERATED_BOT_SHORTCUT_COUNT: usize = {};\n\nimpl Bot {{\n",
        objects.len() - CORE_BOT_SHORTCUTS.len()
    ));

    for object in objects {
        if CORE_BOT_SHORTCUTS.contains(&object.name.as_str()) {
            continue;
        }

        let method_name = object.name.to_snake_case();
        let rust_name = object.name.to_upper_camel_case();
        let required: Vec<_> = object
            .annotations
            .iter()
            .filter(|field| {
                field.required && field_default(object, field, python_fields, enums).is_none()
            })
            .collect();
        let python_path = upstream
            .join("aiogram/methods")
            .join(format!("{}.py", object.name.to_snake_case()));
        let returning = python_return_type(&extract_return_type(&python_path)?, type_names);

        push_doc(&mut output, &object.description);
        let accepts_payload = required.is_empty() && !object.annotations.is_empty();
        output.push_str(&format!("    pub async fn {method_name}(&self"));
        if accepts_payload {
            output.push_str(&format!(", method: crate::methods::{rust_name}"));
        } else {
            for annotation in &required {
                let (field_name, _) = field_name(&annotation.name);
                let field_type = effective_field_type(
                    object,
                    annotation,
                    type_names,
                    Destination::Methods,
                    python_fields,
                );
                output.push_str(&format!(
                    ", {field_name}: {}",
                    ergonomic_argument(&field_type)
                ));
            }
        }
        output.push_str(&format!(") -> Result<{returning}> {{\n"));
        if accepts_payload {
            output.push_str("        self.execute(&method).await\n");
        } else {
            output.push_str(&format!(
                "        self.execute(&crate::methods::{rust_name}::new("
            ));
            for (index, annotation) in required.iter().enumerate() {
                if index > 0 {
                    output.push_str(", ");
                }
                output.push_str(&field_name(&annotation.name).0);
            }
            output.push_str(")).await\n");
        }
        output.push_str("    }\n\n");
    }
    output.push_str("}\n");
    Ok(output)
}

#[derive(Debug)]
enum BoundFillValue {
    Value(String),
    Optional(String),
    None,
}

fn generate_bound_shortcuts(
    aliases: &BoundAliases,
    methods: &[ApiObject],
    type_names: &BTreeSet<String>,
    python_fields: &PythonFields,
    enums: &[EnumDef],
) -> Result<String, Box<dyn std::error::Error>> {
    let count = aliases.values().map(BTreeMap::len).sum::<usize>();
    let method_map: BTreeMap<&str, &ApiObject> = methods
        .iter()
        .map(|method| (method.name.as_str(), method))
        .collect();
    let mut output = generated_header("aiogram-style bound method builders on Telegram objects");
    output.push_str(&format!(
        "/// Number of bound object helpers declared by the pinned aiogram aliases.\npub const BOUND_METHOD_COUNT: usize = {count};\n\n"
    ));

    for (type_name, type_aliases) in aliases {
        output.push_str(&format!("impl super::{type_name} {{\n"));
        for (alias_name, alias) in type_aliases {
            let method = method_map.get(alias.method.as_str()).ok_or_else(|| {
                format!(
                    "bound alias {type_name}.{alias_name} references missing method {}",
                    alias.method
                )
            })?;
            let method_type = method.name.to_upper_camel_case();
            let function_name = bound_function_name(alias_name);
            let required: Vec<_> = method
                .annotations
                .iter()
                .filter(|field| {
                    field.required && field_default(method, field, python_fields, enums).is_none()
                })
                .collect();
            for field in &required {
                if alias.ignore.contains(&field.name) && !alias.fill.contains_key(&field.name) {
                    return Err(format!(
                        "bound alias {type_name}.{alias_name} ignores required method field {}",
                        field.name
                    )
                    .into());
                }
            }
            let user_required: Vec<_> = required
                .iter()
                .copied()
                .filter(|field| {
                    !alias.fill.contains_key(&field.name) && !alias.ignore.contains(&field.name)
                })
                .collect();

            output.push_str(&format!(
                "    #[doc = {:?}]\n",
                format!(
                    "Builds `{method_type}` and fills fields bound by aiogram's `{type_name}.{alias_name}` shortcut."
                )
            ));
            output.push_str(&format!("    pub fn {function_name}(&self"));
            for field in &user_required {
                let (name, _) = field_name(&field.name);
                let field_type = effective_field_type(
                    method,
                    field,
                    type_names,
                    Destination::Methods,
                    python_fields,
                );
                output.push_str(&format!(", {name}: {}", ergonomic_argument(&field_type)));
            }
            output.push_str(&format!(
                ") -> crate::Result<crate::methods::{method_type}> {{\n"
            ));
            // Reading `code` keeps the generated inventory tied to aliases that
            // contain upstream assertion blocks. Missing values are represented
            // as `Result` errors by `bound_fill_value` below.
            let _has_upstream_assertion = alias.code.is_some();
            output.push_str(&format!(
                "        let mut method = crate::methods::{method_type}::new("
            ));
            for (index, field) in required.iter().enumerate() {
                if index > 0 {
                    output.push_str(", ");
                }
                if let Some(raw) = alias.fill.get(&field.name) {
                    match bound_fill_value(type_name, raw)? {
                        BoundFillValue::Value(value) => output.push_str(&value),
                        BoundFillValue::Optional(value) => {
                            let message = format!(
                                "{}.{} requires source field for {}",
                                type_name, alias_name, field.name
                            );
                            output.push_str(&format!(
                                "{value}.ok_or_else(|| crate::Error::InvalidPayload({message:?}.to_owned()))?"
                            ));
                        }
                        BoundFillValue::None => {
                            return Err(format!(
                                "bound alias {type_name}.{alias_name} fills required {} with None",
                                field.name
                            )
                            .into());
                        }
                    }
                } else {
                    output.push_str(&field_name(&field.name).0);
                }
            }
            output.push_str(");\n");

            for (fill_name, raw) in &alias.fill {
                let field = method
                    .annotations
                    .iter()
                    .find(|field| field.name == *fill_name);
                let Some(field) = field else {
                    // aiogram deliberately permits extra method fields. Keep
                    // those aliases byte-for-byte compatible through the
                    // generated payload's flattened `extra` map.
                    match bound_fill_value(type_name, raw)? {
                        BoundFillValue::Value(value) => output.push_str(&format!(
                            "        method.extra.insert({fill_name:?}.to_owned(), serde_json::to_value({value})?);\n"
                        )),
                        BoundFillValue::Optional(value) => output.push_str(&format!(
                            "        if let Some(value) = {value} {{ method.extra.insert({fill_name:?}.to_owned(), serde_json::to_value(value)?); }}\n"
                        )),
                        BoundFillValue::None => {}
                    }
                    continue;
                };
                if field.required {
                    continue;
                }
                let builder = field_name(fill_name).0;
                match bound_fill_value(type_name, raw)? {
                    BoundFillValue::Value(value) => output.push_str(&format!(
                        "        method = method.{builder}({value});\n"
                    )),
                    BoundFillValue::Optional(value) => output.push_str(&format!(
                        "        if let Some(value) = {value} {{ method = method.{builder}(value); }}\n"
                    )),
                    BoundFillValue::None => {}
                }
            }
            output.push_str("        Ok(method)\n    }\n\n");
        }
        output.push_str("}\n\n");
    }
    Ok(output)
}

fn bound_function_name(value: &str) -> String {
    let value = value.to_snake_case();
    if matches!(value.as_str(), "do" | "try" | "yield" | "gen") {
        format!("r#{value}")
    } else {
        value
    }
}

fn bound_fill_value(
    source_type: &str,
    value: &str,
) -> Result<BoundFillValue, Box<dyn std::error::Error>> {
    let value = match value.trim() {
        "None" => BoundFillValue::None,
        "self.chat.id" => BoundFillValue::Value("self.chat.id".to_owned()),
        "self.id" if matches!(source_type, "Chat" | "User") => {
            BoundFillValue::Value("self.id".to_owned())
        }
        "self.id" => BoundFillValue::Value("self.id.clone()".to_owned()),
        "self.user_chat_id" => BoundFillValue::Value("self.user_chat_id".to_owned()),
        "self.from_user.id" => BoundFillValue::Value("self.from_user.id".to_owned()),
        "self.file_id" => BoundFillValue::Value("self.file_id.clone()".to_owned()),
        "self.message_id" => BoundFillValue::Value("self.message_id".to_owned()),
        "self.business_connection_id" => {
            BoundFillValue::Optional("self.business_connection_id.clone()".to_owned())
        }
        "self.message_thread_id if self.is_topic_message else None" => BoundFillValue::Optional(
            "if self.is_topic_message == Some(true) { self.message_thread_id } else { None }"
                .to_owned(),
        ),
        "self.from_user.id if self.ephemeral_message_id and self.from_user else None" => {
            BoundFillValue::Optional(
                "self.from_user.as_ref().filter(|_| self.ephemeral_message_id.is_some()).map(|user| user.id)"
                    .to_owned(),
            )
        }
        "self.as_reply_parameters()" => {
            BoundFillValue::Value("self.as_reply_parameters()".to_owned())
        }
        "self.query_id" => BoundFillValue::Optional("self.query_id.clone()".to_owned()),
        "self.guest_query_id" => {
            BoundFillValue::Optional("self.guest_query_id.clone()".to_owned())
        }
        "self.receiver_user.id" => BoundFillValue::Optional(
            "self.receiver_user.as_ref().map(|user| user.id)".to_owned(),
        ),
        "self.ephemeral_message_id" => {
            BoundFillValue::Optional("self.ephemeral_message_id".to_owned())
        }
        unknown => return Err(format!("unsupported bound fill expression: {unknown}").into()),
    };
    Ok(value)
}

fn generated_header(what: &str) -> String {
    format!(
        "// @generated by `cargo run -p xtask -- generate`. DO NOT EDIT.\n//! Generated {what} for the pinned compatibility baseline.\n#![allow(clippy::too_many_arguments, unused_mut, rustdoc::bare_urls, rustdoc::invalid_html_tags)]\n\n"
    )
}

fn push_doc(output: &mut String, description: &str) {
    let summary = description.lines().next().unwrap_or_default().trim();
    if !summary.is_empty() {
        output.push_str(&format!("#[doc = {:?}]\n", summary));
    }
}

fn push_field(
    output: &mut String,
    object: &ApiObject,
    annotation: &Annotation,
    type_names: &BTreeSet<String>,
    destination: Destination,
    python_fields: &PythonFields,
) {
    let (field_name, rename) = field_name(&annotation.name);
    let mut field_type =
        effective_field_type(object, annotation, type_names, destination, python_fields);
    if !annotation.required {
        field_type = format!("Option<{field_type}>");
    }
    if !annotation.description.is_empty() {
        let summary = annotation
            .description
            .lines()
            .next()
            .unwrap_or_default()
            .trim();
        output.push_str(&format!("    #[doc = {:?}]\n", summary));
    }
    if let Some(rename) = rename {
        output.push_str(&format!("    #[serde(rename = {:?}", rename));
        if !annotation.required {
            output.push_str(", default, skip_serializing_if = \"Option::is_none\"");
        }
        output.push_str(")]\n");
    } else if !annotation.required {
        output.push_str("    #[serde(default, skip_serializing_if = \"Option::is_none\")]\n");
    }
    output.push_str(&format!("    pub {field_name}: {field_type},\n"));
}

fn push_constructor(
    output: &mut String,
    object: &ApiObject,
    type_names: &BTreeSet<String>,
    destination: Destination,
    python_fields: &PythonFields,
    enums: &[EnumDef],
) {
    let rust_name = match destination {
        Destination::Methods => object.name.to_upper_camel_case(),
        _ => object.name.clone(),
    };
    let required: Vec<_> = object
        .annotations
        .iter()
        .filter(|field| {
            field.required && field_default(object, field, python_fields, enums).is_none()
        })
        .collect();
    output.push_str(&format!("impl {rust_name} {{\n    pub fn new("));
    for (index, annotation) in required.iter().enumerate() {
        if index > 0 {
            output.push_str(", ");
        }
        let (field_name, _) = field_name(&annotation.name);
        let field_type =
            effective_field_type(object, annotation, type_names, destination, python_fields);
        output.push_str(&format!(
            "{field_name}: {}",
            ergonomic_argument(&field_type)
        ));
    }
    output.push_str(") -> Self {\n        Self {\n");
    for annotation in &object.annotations {
        let (field_name, _) = field_name(&annotation.name);
        if annotation.required && field_default(object, annotation, python_fields, enums).is_none()
        {
            let field_type =
                effective_field_type(object, annotation, type_names, destination, python_fields);
            if is_into_argument(&field_type) {
                output.push_str(&format!("            {field_name}: {field_name}.into(),\n"));
            } else {
                output.push_str(&format!("            {field_name},\n"));
            }
        } else if let Some(default) = field_default(object, annotation, python_fields, enums) {
            output.push_str(&format!("            {field_name}: {default},\n"));
        } else {
            output.push_str(&format!("            {field_name}: None,\n"));
        }
    }
    output.push_str("            extra: BTreeMap::new(),\n        }\n    }\n\n");
    for annotation in object.annotations.iter().filter(|field| !field.required) {
        let (field_name, _) = field_name(&annotation.name);
        let field_type =
            effective_field_type(object, annotation, type_names, destination, python_fields);
        output.push_str(&format!(
            "    pub fn {field_name}(mut self, value: {}) -> Self {{\n",
            ergonomic_argument(&field_type)
        ));
        if is_into_argument(&field_type) {
            output.push_str(&format!(
                "        self.{field_name} = Some(value.into());\n"
            ));
        } else {
            output.push_str(&format!("        self.{field_name} = Some(value);\n"));
        }
        output.push_str("        self\n    }\n\n");
    }
    output.push_str("}\n\n");
    if required.is_empty() {
        output.push_str(&format!(
            "impl Default for {rust_name} {{\n    fn default() -> Self {{ Self::new() }}\n}}\n\n"
        ));
    }
}

fn effective_field_type(
    object: &ApiObject,
    annotation: &Annotation,
    type_names: &BTreeSet<String>,
    destination: Destination,
    python_fields: &PythonFields,
) -> String {
    python_fields
        .get(&(object.name.clone(), annotation.name.clone()))
        .and_then(|field| python_annotation_type(&field.annotation, type_names, destination, true))
        .unwrap_or_else(|| rust_type(&annotation.kind, type_names, destination, true))
}

fn python_annotation_type(
    raw: &str,
    type_names: &BTreeSet<String>,
    destination: Destination,
    direct: bool,
) -> Option<String> {
    let raw = raw.trim();
    if let Some(inner) = raw
        .strip_prefix("list[")
        .and_then(|value| value.strip_suffix(']'))
    {
        return python_annotation_type(inner, type_names, destination, false)
            .map(|inner| format!("Vec<{inner}>"));
    }
    if raw.starts_with("dict[") {
        let value = if raw.contains("InputFile") {
            type_path("InputFile", type_names, destination)
        } else {
            "Value".to_owned()
        };
        return Some(format!("BTreeMap<String, {value}>"));
    }
    if let Some(literal) = raw
        .strip_prefix("Literal[")
        .and_then(|value| value.strip_suffix(']'))
    {
        return Some(if literal == "True" {
            "bool".to_owned()
        } else if literal.parse::<i64>().is_ok() {
            "i64".to_owned()
        } else {
            "String".to_owned()
        });
    }

    let variants: Vec<_> = raw
        .split(" | ")
        .filter(|variant| !matches!(*variant, "None" | "Default"))
        .collect();
    if variants.len() > 1 {
        if variants.iter().any(|variant| variant.contains("InputFile"))
            && variants
                .iter()
                .all(|variant| matches!(*variant, "str" | "InputFile" | "InputFileUnion"))
        {
            return Some(type_path("InputFile", type_names, destination));
        }
        if variants.iter().all(|variant| {
            matches!(
                *variant,
                "int" | "DateTime" | "DateTimeUnion" | "datetime.datetime" | "datetime.timedelta"
            )
        }) {
            return Some("i64".to_owned());
        }
        return None;
    }
    let raw = variants.first().copied().unwrap_or(raw);
    let scalar = match raw {
        "str" => "String".to_owned(),
        "int" | "DateTime" | "DateTimeUnion" | "datetime.datetime" | "datetime.timedelta" => {
            "i64".to_owned()
        }
        "float" => "f64".to_owned(),
        "bool" => "bool".to_owned(),
        "Any" | "TelegramType" => "Value".to_owned(),
        "ChatIdUnion" => type_path("ChatId", type_names, destination),
        "InputFile" | "InputFileUnion" => type_path("InputFile", type_names, destination),
        name if type_names.contains(name) || is_generated_union(name) => {
            let path = type_path(name, type_names, destination);
            if matches!(destination, Destination::Types)
                && direct
                && matches!(name, "Message" | "Chat" | "RichTextUnion")
            {
                format!("Box<{path}>")
            } else {
                path
            }
        }
        _ => return None,
    };
    Some(scalar)
}

fn field_default(
    object: &ApiObject,
    annotation: &Annotation,
    python_fields: &PythonFields,
    enums: &[EnumDef],
) -> Option<String> {
    let default = python_fields
        .get(&(object.name.clone(), annotation.name.clone()))?
        .default
        .as_deref()?
        .trim();
    if default == "True" {
        return Some("true".to_owned());
    }
    if default == "False" {
        return Some("false".to_owned());
    }
    if default.parse::<i64>().is_ok() {
        return Some(default.to_owned());
    }
    if default.starts_with('"') && default.ends_with('"') {
        let value: String = serde_json::from_str(default).ok()?;
        return Some(format!("{:?}.to_owned()", value));
    }
    let (enum_name, variant_name) = default.split_once('.')?;
    let definition = enums
        .iter()
        .find(|definition| definition.name == enum_name)?;
    let variant = definition
        .variants
        .iter()
        .find(|variant| variant.name == variant_name.to_upper_camel_case())?;
    Some(match definition.kind {
        EnumKind::String => format!("{:?}.to_owned()", variant.value),
        EnumKind::Integer => variant.value.clone(),
    })
}

fn ergonomic_argument(field_type: &str) -> String {
    if is_into_argument(field_type) {
        format!("impl Into<{field_type}>")
    } else {
        field_type.to_owned()
    }
}

fn is_into_argument(field_type: &str) -> bool {
    matches!(field_type, "String" | "super::ChatId" | "super::InputFile")
        || matches!(
            field_type,
            "crate::types::ChatId" | "crate::types::InputFile"
        )
        || field_type
            .rsplit("::")
            .next()
            .is_some_and(|name| name.ends_with("Union"))
}

fn push_collect_files_impl(output: &mut String, object: &ApiObject, destination: Destination) {
    let rust_name = match destination {
        Destination::Methods => object.name.to_upper_camel_case(),
        _ => object.name.clone(),
    };
    let output_name = if object.annotations.is_empty() {
        "_output"
    } else {
        "output"
    };
    output.push_str(&format!(
        "impl CollectFiles for {rust_name} {{\n    fn collect_files(&self, {output_name}: &mut Vec<InputFileUpload>) {{\n"
    ));
    for annotation in &object.annotations {
        let (field_name, _) = field_name(&annotation.name);
        output.push_str(&format!(
            "        CollectFiles::collect_files(&self.{field_name}, output);\n"
        ));
    }
    output.push_str("    }\n}\n\n");
}

fn rust_type(
    raw: &str,
    type_names: &BTreeSet<String>,
    destination: Destination,
    direct: bool,
) -> String {
    let raw = raw.trim();
    if raw
        == "Array of InputMediaAudio, InputMediaDocument, InputMediaLivePhoto, InputMediaPhoto and InputMediaVideo"
    {
        return format!("Vec<{}>", type_path("MediaUnion", type_names, destination));
    }
    if raw
        == "InputMediaAnimation or InputMediaAudio or InputMediaPhoto or InputMediaVideo or InputMediaVoiceNote"
    {
        return type_path("InputRichMessageMediaUnion", type_names, destination);
    }
    if raw == "InlineKeyboardMarkup or ReplyKeyboardMarkup or ReplyKeyboardRemove or ForceReply" {
        return type_path("ReplyMarkupUnion", type_names, destination);
    }
    if let Some(inner) = raw.strip_prefix("Array of ") {
        return format!("Vec<{}>", rust_type(inner, type_names, destination, false));
    }
    match raw {
        "Integer" => return "i64".to_owned(),
        "Float" | "Float number" => return "f64".to_owned(),
        "String" => return "String".to_owned(),
        "Boolean" | "True" => return "bool".to_owned(),
        "Integer or String" => return type_path("ChatId", type_names, destination),
        "InputFile" | "InputFile or String" => {
            return type_path("InputFile", type_names, destination);
        }
        _ => {}
    }

    if let Some(union) = union_for_base(raw) {
        let path = type_path(union, type_names, destination);
        if matches!(destination, Destination::Types) && direct && union == "RichTextUnion" {
            return format!("Box<{path}>");
        }
        return path;
    }

    if raw.contains(" or ") || raw.contains(", ") || raw.contains(" and ") {
        return "Value".to_owned();
    }
    if type_names.contains(raw) || CORE_TYPES.contains(&raw) {
        let path = type_path(raw, type_names, destination);
        if matches!(destination, Destination::Types) && direct && matches!(raw, "Message" | "Chat")
        {
            return format!("Box<{path}>");
        }
        return path;
    }
    "Value".to_owned()
}

fn type_path(name: &str, type_names: &BTreeSet<String>, destination: Destination) -> String {
    match destination {
        Destination::Types if matches!(name, "ChatId" | "InputFile") => {
            format!("super::{name}")
        }
        Destination::Types if type_names.contains(name) => name.to_owned(),
        Destination::Types if is_generated_union(name) => name.to_owned(),
        Destination::Methods | Destination::Return => format!("crate::types::{name}"),
        _ => "Value".to_owned(),
    }
}

fn python_return_type(raw: &str, type_names: &BTreeSet<String>) -> String {
    let raw = raw.trim();
    if let Some(inner) = raw
        .strip_prefix("list[")
        .and_then(|value| value.strip_suffix(']'))
    {
        return format!("Vec<{}>", python_return_type(inner, type_names));
    }
    if raw == "Message | bool" {
        return "crate::methods::MessageOrBool".to_owned();
    }
    if raw.contains('|') {
        return "Value".to_owned();
    }
    match raw {
        "bool" => "bool".to_owned(),
        "int" => "i64".to_owned(),
        "float" => "f64".to_owned(),
        "str" => "String".to_owned(),
        "Any" => "Value".to_owned(),
        custom
            if type_names.contains(custom)
                || CORE_TYPES.contains(&custom)
                || is_generated_union(custom) =>
        {
            type_path(custom, type_names, Destination::Return)
        }
        _ => "Value".to_owned(),
    }
}

fn union_for_base(name: &str) -> Option<&'static str> {
    match name {
        "BackgroundFill" => Some("BackgroundFillUnion"),
        "BackgroundType" => Some("BackgroundTypeUnion"),
        "BotCommandScope" => Some("BotCommandScopeUnion"),
        "ChatBoostSource" => Some("ChatBoostSourceUnion"),
        "ChatMember" => Some("ChatMemberUnion"),
        "InlineQueryResult" => Some("InlineQueryResultUnion"),
        "InputMedia" => Some("InputMediaUnion"),
        "InputMessageContent" => Some("InputMessageContentUnion"),
        "InputPaidMedia" => Some("InputPaidMediaUnion"),
        "InputPollMedia" => Some("InputPollMediaUnion"),
        "InputPollOption" => Some("InputPollOptionUnion"),
        "InputPollOptionMedia" => Some("InputPollOptionMediaUnion"),
        "InputProfilePhoto" => Some("InputProfilePhotoUnion"),
        "InputRichBlock" => Some("InputRichBlockUnion"),
        "InputRichMessageMedia" => Some("InputRichMessageMediaUnion"),
        "InputStoryContent" => Some("InputStoryContentUnion"),
        "MaybeInaccessibleMessage" => Some("MaybeInaccessibleMessageUnion"),
        "MenuButton" => Some("MenuButtonUnion"),
        "MessageOrigin" => Some("MessageOriginUnion"),
        "OwnedGift" => Some("OwnedGiftUnion"),
        "PaidMedia" => Some("PaidMediaUnion"),
        "PassportElementError" => Some("PassportElementErrorUnion"),
        "ReactionType" => Some("ReactionTypeUnion"),
        "RevenueWithdrawalState" => Some("RevenueWithdrawalStateUnion"),
        "RichBlock" => Some("RichBlockUnion"),
        "RichText" => Some("RichTextUnion"),
        "StoryAreaType" => Some("StoryAreaTypeUnion"),
        "TransactionPartner" => Some("TransactionPartnerUnion"),
        _ => None,
    }
}

fn is_generated_union(name: &str) -> bool {
    matches!(
        name,
        "BackgroundFillUnion"
            | "BackgroundTypeUnion"
            | "BotCommandScopeUnion"
            | "ChatBoostSourceUnion"
            | "ChatIdUnion"
            | "ChatMemberUnion"
            | "DateTimeUnion"
            | "InlineQueryResultUnion"
            | "InputFileUnion"
            | "InputMediaUnion"
            | "InputMessageContentUnion"
            | "InputPaidMediaUnion"
            | "InputPollMediaUnion"
            | "InputPollOptionMediaUnion"
            | "InputPollOptionUnion"
            | "InputProfilePhotoUnion"
            | "InputRichBlockUnion"
            | "InputRichMessageMediaUnion"
            | "InputStoryContentUnion"
            | "MaybeInaccessibleMessageUnion"
            | "MediaUnion"
            | "MenuButtonUnion"
            | "MessageOriginUnion"
            | "OwnedGiftUnion"
            | "PaidMediaUnion"
            | "PassportElementErrorUnion"
            | "ReactionTypeUnion"
            | "ReplyMarkupUnion"
            | "ResultChatMemberUnion"
            | "ResultMenuButtonUnion"
            | "RevenueWithdrawalStateUnion"
            | "RichBlockUnion"
            | "RichTextUnion"
            | "StoryAreaTypeUnion"
            | "TransactionPartnerUnion"
    )
}

fn extract_return_type(path: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let source = fs::read_to_string(path)?;
    source
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("__returning__ = ")
                .map(str::to_owned)
        })
        .ok_or_else(|| format!("return type not found in {}", path.display()).into())
}

fn field_name(name: &str) -> (String, Option<&str>) {
    match name {
        "from" => ("from_user".to_owned(), Some("from")),
        "bot" => ("bot_user".to_owned(), Some("bot")),
        "type" => ("kind".to_owned(), Some("type")),
        keyword if is_keyword(keyword) => (format!("{keyword}_"), Some(keyword)),
        ordinary => (ordinary.to_snake_case(), None),
    }
}

fn is_keyword(value: &str) -> bool {
    matches!(
        value,
        "as" | "async"
            | "await"
            | "break"
            | "const"
            | "continue"
            | "crate"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
    )
}
