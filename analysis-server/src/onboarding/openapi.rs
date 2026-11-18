//! Source-derived OpenAPI contracts. Uses rgctl's Java symbols, annotations and
//! cached native tree; never runs the application or loads Java classes.
use super::model::Operation;
use anyhow::Result;
use rgctl_analysis::cfg_builder::ParsedSourceFile;
use rgctl_plugin_api::{
    ExtractAllResult, LanguagePlugin, Relation, RelationType, Symbol, SymbolType,
};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

pub struct Unit {
    pub source: Vec<u8>,
    pub extracted: ExtractAllResult,
    pub parsed: ParsedSourceFile,
}
pub struct JavaSchemas {
    pub units: BTreeMap<String, Unit>,
    types: BTreeMap<String, (String, Symbol)>,
}
#[derive(Clone, Debug)]
struct Type {
    name: String,
    args: Vec<Type>,
}
fn split_args(raw: &str) -> Vec<&str> {
    let mut depth = 0;
    let mut start = 0;
    let mut result = vec![];
    for (i, c) in raw.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => {
                result.push(raw[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    if !raw[start..].trim().is_empty() {
        result.push(raw[start..].trim());
    }
    result
}
fn parse_type(raw: &str) -> Type {
    let raw = raw.trim();
    if let Some(inner) = raw.strip_suffix("[]").or_else(|| raw.strip_suffix("...")) {
        return Type {
            name: "[]".into(),
            args: vec![parse_type(inner)],
        };
    }
    let raw = raw.strip_prefix("? extends ").unwrap_or(raw);
    if let Some(i) = raw.find('<') {
        return Type {
            name: raw[..i].trim().into(),
            args: split_args(raw[i + 1..].strip_suffix('>').unwrap_or(&raw[i + 1..]))
                .into_iter()
                .map(parse_type)
                .collect(),
        };
    }
    Type {
        name: raw.into(),
        args: vec![],
    }
}
fn ann_name(r: &Relation) -> &str {
    r.to.rsplit('.').next().unwrap_or(&r.to)
}
fn arg(r: &Relation, key: &str) -> Option<String> {
    r.metadata["annotation_args"].get(key).and_then(|v| {
        v.as_str()
            .map(str::to_owned)
            .or_else(|| (!v.is_null()).then(|| v.to_string()))
    })
}
fn named(r: &Relation) -> Option<String> {
    arg(r, "name")
        .or_else(|| arg(r, "value"))
        .filter(|s| !s.is_empty())
}
fn annotations<'a>(unit: &'a Unit, owner: &str) -> Vec<&'a Relation> {
    unit.extracted
        .relations
        .iter()
        .filter(|r| {
            r.relation_type == RelationType::AnnotatedWith
                && r.from == owner
                && r.metadata["annotation_scope"] != "Parameter"
        })
        .filter(|r| known_annotation(unit, r))
        .collect()
}
fn annotation_package(name: &str) -> bool {
    [
        "org.springframework.web.bind.annotation.",
        "jakarta.ws.rs.",
        "javax.ws.rs.",
        "jakarta.validation.constraints.",
        "javax.validation.constraints.",
        "com.fasterxml.jackson.annotation.",
        "com.fasterxml.jackson.databind.annotation.",
        "io.swagger.v3.oas.annotations.media.",
    ]
    .iter()
    .any(|p| name.starts_with(p))
}
fn imports(unit: &Unit) -> impl Iterator<Item = &str> {
    unit.extracted
        .symbols
        .iter()
        .filter(|s| s.symbol_type == SymbolType::Import)
        .filter_map(|s| {
            s.name
                .trim()
                .strip_prefix("import ")
                .and_then(|n| n.trim().strip_suffix(';'))
        })
}
fn known_annotation(unit: &Unit, r: &Relation) -> bool {
    let hint = r
        .to_qualified_hint
        .as_deref()
        .or_else(|| r.metadata["fqn"].as_str())
        .unwrap_or(&r.to);
    if annotation_package(hint) {
        return true;
    }
    if r.to.contains('.')
        || unit
            .extracted
            .symbols
            .iter()
            .any(|s| s.symbol_type == SymbolType::Annotation && s.name == r.to)
    {
        return false;
    }
    if let Some(explicit) = imports(unit).find(|i| i.rsplit('.').next() == Some(&r.to)) {
        return annotation_package(explicit);
    }
    imports(unit)
        .filter_map(|i| i.strip_suffix(".*"))
        .any(|p| annotation_package(&format!("{p}.{}", r.to)))
}
fn method_annotations<'a>(unit: &'a Unit, method: &Symbol) -> Vec<&'a Relation> {
    annotations(unit, method.qualified_name.as_deref().unwrap_or(""))
        .into_iter()
        .filter(|r| {
            r.metadata["annotation_scope"] == "Method"
                && r.call_site_start_byte.is_some_and(|b| {
                    method
                        .start_byte
                        .zip(method.end_byte)
                        .is_some_and(|(s, e)| (s..e).contains(&b))
                })
        })
        .collect()
}
fn resolve_name(unit: &Unit, name: &str) -> String {
    if let Some((first, rest)) = name.split_once('.') {
        if unit.extracted.symbols.iter().any(|s| {
            s.name == first && matches!(s.symbol_type, SymbolType::Class | SymbolType::Interface)
        }) || imports(unit).any(|i| i.rsplit('.').next() == Some(first))
        {
            return format!("{}.{}", resolve_name(unit, first), rest);
        }
    }
    if name.contains('.')
        || matches!(
            name,
            "[]" | "void"
                | "boolean"
                | "byte"
                | "short"
                | "int"
                | "long"
                | "float"
                | "double"
                | "char"
                | "?"
        )
    {
        return name.into();
    }
    if let Some(s) = unit.extracted.symbols.iter().find(|s| {
        s.name == name
            && matches!(
                s.symbol_type,
                SymbolType::Class | SymbolType::Interface | SymbolType::Enum
            )
    }) {
        return s.qualified_name.clone().unwrap_or_else(|| name.into());
    }
    for r in unit
        .extracted
        .relations
        .iter()
        .filter(|r| r.metadata["import"] == true)
    {
        if let Some(fqn) = r.metadata["fqn"].as_str() {
            if fqn.rsplit('.').next() == Some(name) {
                return fqn.into();
            }
        }
    }
    match name {
        "String" | "Boolean" | "Byte" | "Short" | "Integer" | "Long" | "Float" | "Double"
        | "Character" | "Object" | "Void" => return format!("java.lang.{name}"),
        _ => {}
    }
    // Resolve wildcard imports only when they identify a known Java/API type.
    for r in unit
        .extracted
        .relations
        .iter()
        .filter(|r| r.metadata["import"] == true)
    {
        if let Some(package) = r.metadata["fqn"].as_str() {
            if let Some(end) = r
                .call_site_start_byte
                .zip(r.call_site_byte_len)
                .map(|(s, n)| s as usize + n as usize)
            {
                if unit
                    .source
                    .get(end..)
                    .is_some_and(|s| String::from_utf8_lossy(s).trim_start().starts_with(".*"))
                {
                    let candidate = format!("{package}.{name}");
                    if builtin(&candidate).is_some() || container(&candidate) || wrapper(&candidate)
                    {
                        return candidate;
                    }
                }
            }
        }
    }
    let package = unit
        .extracted
        .symbols
        .iter()
        .filter_map(|s| s.qualified_name.as_deref())
        .find_map(|q| q.rsplit_once('.').map(|(p, _)| p));
    package
        .map(|p| format!("{p}.{name}"))
        .unwrap_or_else(|| name.into())
}
fn builtin(name: &str) -> Option<Value> {
    Some(match name {
        "String" | "java.lang.String" | "char" | "java.lang.Character" => json!({"type":"string"}),
        "boolean" | "java.lang.Boolean" => json!({"type":"boolean"}),
        "byte" | "short" | "int" | "java.lang.Byte" | "java.lang.Short" | "java.lang.Integer" => {
            json!({"type":"integer","format":"int32"})
        }
        "long" | "java.lang.Long" => json!({"type":"integer","format":"int64"}),
        "float" | "java.lang.Float" => json!({"type":"number","format":"float"}),
        "double" | "java.lang.Double" => json!({"type":"number","format":"double"}),
        "java.math.BigDecimal" | "java.math.BigInteger" => {
            json!({"type":if name.ends_with("BigInteger"){"integer"}else{"number"}})
        }
        "java.util.UUID" => json!({"type":"string","format":"uuid"}),
        "java.time.LocalDate" => json!({"type":"string","format":"date"}),
        "java.time.Instant"
        | "java.time.OffsetDateTime"
        | "java.time.ZonedDateTime"
        | "java.util.Date" => json!({"type":"string","format":"date-time"}),
        "java.time.LocalDateTime" | "java.time.LocalTime" => {
            json!({"type":"string","x-java-type":name})
        }
        "java.lang.Object" => json!({}),
        _ => return None,
    })
}
fn container(n: &str) -> bool {
    matches!(
        n,
        "java.util.List"
            | "java.util.Set"
            | "java.util.Collection"
            | "java.lang.Iterable"
            | "java.util.Map"
            | "reactor.core.publisher.Flux"
            | "[]"
    )
}
fn wrapper(n: &str) -> bool {
    matches!(
        n,
        "org.springframework.http.ResponseEntity"
            | "org.springframework.http.HttpEntity"
            | "java.util.Optional"
            | "reactor.core.publisher.Mono"
            | "java.util.concurrent.CompletionStage"
            | "java.util.concurrent.CompletableFuture"
    )
}
fn canonical(t: &Type) -> String {
    if t.args.is_empty() {
        t.name.clone()
    } else {
        format!(
            "{}<{}>",
            t.name,
            t.args.iter().map(canonical).collect::<Vec<_>>().join(",")
        )
    }
}
fn decap(s: &str) -> String {
    if s.chars().take(2).all(char::is_uppercase) {
        s.into()
    } else {
        let mut c = s.chars();
        c.next()
            .map(|a| format!("{}{}", a.to_lowercase(), c.as_str()))
            .unwrap_or_default()
    }
}
fn getter(s: &Symbol) -> Option<String> {
    if s.return_type.as_deref().is_none_or(|t| t == "void") {
        return None;
    }
    if !s.parameters.is_empty() || s.modifiers.iter().any(|m| m == "static") {
        return None;
    }
    s.name
        .strip_prefix("get")
        .filter(|n| !n.is_empty())
        .or_else(|| {
            s.name.strip_prefix("is").filter(|_| {
                matches!(
                    s.return_type.as_deref(),
                    Some("boolean" | "Boolean" | "java.lang.Boolean")
                )
            })
        })
        .map(decap)
}
impl JavaSchemas {
    pub fn read(root: &Path, files: &[PathBuf]) -> Result<Self> {
        let java = rgctl_lang_java::JavaPlugin::new()?;
        let mut units = BTreeMap::new();
        let mut types = BTreeMap::new();
        for relative in files {
            let key = relative.to_string_lossy().replace('\\', "/");
            let source = fs::read(root.join(relative))?;
            let extracted = java.extract_all(&root.join(relative), &source)?;
            for s in &extracted.symbols {
                if matches!(
                    s.symbol_type,
                    SymbolType::Class | SymbolType::Interface | SymbolType::Enum
                ) {
                    if let Some(q) = &s.qualified_name {
                        types
                            .entry(q.clone())
                            .or_insert_with(|| (key.clone(), s.clone()));
                    }
                }
            }
            let parsed = ParsedSourceFile::parse("java", &source)?;
            units.insert(
                key,
                Unit {
                    source,
                    extracted,
                    parsed,
                },
            );
        }
        Ok(Self { units, types })
    }
    pub fn contracts(
        &self,
        operations: &mut [Operation],
        diagnostics: &mut Vec<Value>,
    ) -> BTreeMap<String, Value> {
        let mut resolver = Resolver {
            index: self,
            schemas: BTreeMap::new(),
            unknown: BTreeSet::new(),
        };
        for op in operations.iter_mut().filter(|o| o.direction == "provides") {
            if let Some(unit) = self.units.get(&op.symbol.file) {
                if let Some(method) = unit.extracted.symbols.iter().find(|s| {
                    s.qualified_name.as_deref() == Some(&op.symbol.qualified_name)
                        && s.location.start_line == op.symbol.start_line
                }) {
                    op.contract = Some(resolver.operation(unit, method, op));
                }
            }
        }
        for name in resolver.unknown {
            diagnostics.push(json!({"code":"UNRESOLVED_API_SCHEMA","javaType":name}));
        }
        resolver.schemas
    }
}
struct Resolver<'a> {
    index: &'a JavaSchemas,
    schemas: BTreeMap<String, Value>,
    unknown: BTreeSet<String>,
}
impl Resolver<'_> {
    fn resolve(&self, unit: &Unit, t: &Type, vars: &BTreeMap<String, Type>) -> Type {
        if let Some(value) = vars.get(&t.name) {
            return value.clone();
        }
        let mut name = resolve_name(unit, &t.name);
        if !self.index.types.contains_key(&name) && !t.name.contains('.') {
            // Same-package references may live in separate source files.
            let owners: Vec<_> = unit
                .extracted
                .symbols
                .iter()
                .filter_map(|s| s.qualified_name.as_deref())
                .collect();
            for owner in owners {
                if let Some((package, _)) = owner.rsplit_once('.') {
                    let q = format!("{package}.{}", t.name);
                    if self.index.types.contains_key(&q) {
                        name = q;
                        break;
                    }
                }
            }
        }
        if !self.index.types.contains_key(&name) && !t.name.contains('.') {
            let matches: Vec<_> = imports(unit)
                .filter_map(|i| i.strip_suffix(".*"))
                .map(|p| format!("{p}.{}", t.name))
                .filter(|q| self.index.types.contains_key(q))
                .collect();
            if matches.len() == 1 {
                name = matches[0].clone();
            }
        }
        Type {
            name,
            args: t.args.iter().map(|a| self.resolve(unit, a, vars)).collect(),
        }
    }
    fn unknown(&mut self, t: &Type) -> Value {
        let key = canonical(t);
        self.unknown.insert(key.clone());
        json!({"x-java-type":key,"x-schema-unresolved":true})
    }
    fn schema(&mut self, t: &Type, depth: usize) -> Value {
        if depth > 32 || self.schemas.len() > 512 {
            return self.unknown(t);
        }
        if let Some(schema) = builtin(&t.name) {
            return schema;
        }
        if wrapper(&t.name) {
            return t
                .args
                .first()
                .map(|a| self.schema(a, depth + 1))
                .unwrap_or_else(|| self.unknown(t));
        }
        if t.name == "[]" && t.args.first().is_some_and(|a| a.name == "byte") {
            return json!({"type":"string","format":"byte"});
        }
        if container(&t.name) {
            if t.name == "java.util.Map" {
                let item = t
                    .args
                    .get(1)
                    .map(|a| self.schema(a, depth + 1))
                    .unwrap_or_else(|| self.unknown(t));
                return json!({"type":"object","additionalProperties":item});
            }
            let item = t
                .args
                .first()
                .map(|a| self.schema(a, depth + 1))
                .unwrap_or_else(|| self.unknown(t));
            let mut schema = json!({"type":"array","items":item});
            if t.name == "java.util.Set" {
                schema["uniqueItems"] = json!(true);
            }
            return schema;
        }
        let Some((file, class)) = self.index.types.get(&t.name) else {
            return self.unknown(t);
        };
        let unit = &self.index.units[file];
        let class_ann = annotations(unit, &t.name);
        if class_ann.iter().any(|a| {
            matches!(
                ann_name(a),
                "JsonTypeInfo" | "JsonAutoDetect" | "JsonSerialize" | "JsonDeserialize"
            )
        }) || unit
            .extracted
            .relations
            .iter()
            .any(|r| r.from.starts_with(&format!("{}.", t.name)) && ann_name(r) == "JsonValue")
        {
            return self.unknown(t);
        }
        let key = if t.args.is_empty() {
            t.name.clone()
        } else {
            format!(
                "{}__{}",
                t.name,
                &super::store::hash(canonical(t).as_bytes())[..12]
            )
        };
        let reference = json!({"$ref":format!("#/components/schemas/{key}")});
        if self.schemas.contains_key(&key) {
            return reference;
        }
        self.schemas.insert(key.clone(), json!({"type":"object"}));
        if class.symbol_type == SymbolType::Enum {
            self.schemas.insert(key,json!({"type":"string","enum":class.fields.iter().map(|f|f.name.clone()).collect::<Vec<_>>()}));
            return reference;
        }
        let names = class.metadata["type_params"]
            .as_str()
            .unwrap_or("")
            .trim_matches(['<', '>']);
        let vars: BTreeMap<_, _> = split_args(names)
            .iter()
            .zip(&t.args)
            .map(|(n, a)| {
                (
                    n.split_whitespace().next().unwrap_or(n).to_string(),
                    a.clone(),
                )
            })
            .collect();
        let methods: Vec<_> = unit
            .extracted
            .symbols
            .iter()
            .filter(|s| {
                s.qualified_name
                    .as_deref()
                    .and_then(|q| q.rsplit_once('.'))
                    .is_some_and(|(owner, _)| owner == t.name)
            })
            .collect();
        let mut properties = BTreeMap::<String, (String, Vec<&Relation>)>::new();
        for field in &class.fields {
            if field.visibility.as_deref().is_some_and(|v| {
                v.split_whitespace()
                    .any(|m| m == "static" || m == "transient")
            }) {
                continue;
            }
            let anns = annotations(unit, &format!("{}.{}", t.name, field.name));
            let visible = class.metadata["is_record"] == true
                || field
                    .visibility
                    .as_deref()
                    .is_some_and(|v| v.split_whitespace().any(|m| m == "public"))
                || anns.iter().any(|r| ann_name(r) == "JsonProperty")
                || methods.iter().any(|s| {
                    s.modifiers.iter().any(|m| m == "public")
                        && getter(s).as_deref() == Some(&field.name)
                });
            if visible {
                properties.insert(
                    field.name.clone(),
                    (field.field_type.clone().unwrap_or("?".into()), anns),
                );
            }
        }
        for method in methods {
            if let Some(name) = getter(method) {
                let anns = method_annotations(unit, method);
                if method.modifiers.iter().any(|m| m == "public")
                    || class.symbol_type == SymbolType::Interface
                    || anns.iter().any(|a| ann_name(a) == "JsonProperty")
                {
                    let entry = properties.entry(name).or_insert_with(|| {
                        (method.return_type.clone().unwrap_or("?".into()), vec![])
                    });
                    entry.1.extend(anns);
                }
            }
        }
        let ignored: Vec<_> = class_ann
            .iter()
            .filter(|a| ann_name(a) == "JsonIgnoreProperties")
            .collect();
        let mut body = Map::new();
        let mut required = vec![];
        for (name, (ty, anns)) in properties {
            if anns
                .iter()
                .any(|a| ann_name(a) == "JsonIgnore" && arg(a, "value").as_deref() != Some("false"))
            {
                continue;
            }
            let output = anns
                .iter()
                .find(|a| ann_name(a) == "JsonProperty")
                .and_then(|a| named(a))
                .unwrap_or(name);
            let ignore = ignored.iter().find(|a| {
                arg(a, "value").is_some_and(|s| {
                    if s.starts_with('{') {
                        s.trim_matches(['{', '}'])
                            .split(',')
                            .any(|v| v.trim().trim_matches('"') == output)
                    } else {
                        s == output
                    }
                })
            });
            if ignore.is_some_and(|a| {
                arg(a, "allowGetters").as_deref() != Some("true")
                    && arg(a, "allowSetters").as_deref() != Some("true")
            }) {
                continue;
            }
            let resolved = self.resolve(unit, &parse_type(&ty), &vars);
            let mut schema = if anns
                .iter()
                .any(|a| matches!(ann_name(a), "JsonSerialize" | "JsonDeserialize"))
            {
                self.unknown(&resolved)
            } else {
                self.schema(&resolved, depth + 1)
            };
            if let Some(ignore) = ignore {
                if arg(ignore, "allowGetters").as_deref() == Some("true") {
                    schema["readOnly"] = json!(true);
                }
                if arg(ignore, "allowSetters").as_deref() == Some("true") {
                    schema["writeOnly"] = json!(true);
                }
            }
            if constraints(&mut schema, &anns) {
                required.push(output.clone());
            }
            body.insert(output, schema);
        }
        let mut schema = json!({"type":"object","properties":body,"x-java-type":canonical(t)});
        constraints(&mut schema, &class_ann);
        if !required.is_empty() {
            schema["required"] = json!(required);
        }
        // Source-available inheritance is compositional; missing external parents
        // remain explicitly unresolved rather than silently disappearing.
        let parents: Vec<_> = unit
            .extracted
            .relations
            .iter()
            .filter(|r| r.from == t.name && r.relation_type == RelationType::Extends)
            .collect();
        if !parents.is_empty() {
            let mut all = vec![];
            for parent in parents {
                let ty = self.resolve(unit, &parse_type(&parent.to), &vars);
                if !matches!(ty.name.as_str(), "java.lang.Object" | "java.lang.Record") {
                    all.push(self.schema(&ty, depth + 1));
                }
            }
            if !all.is_empty() {
                all.push(schema);
                schema = json!({"allOf":all});
            }
        }
        self.schemas.insert(key, schema);
        reference
    }
    fn operation(&mut self, unit: &Unit, method: &Symbol, op: &Operation) -> Value {
        let owner = method
            .qualified_name
            .as_deref()
            .and_then(|q| q.rsplit_once('.'))
            .map(|(o, _)| o)
            .unwrap_or("");
        let mut anns = annotations(unit, owner);
        anns.extend(method_annotations(unit, method));
        let mut parameters = vec![];
        let mut request_body = None;
        for p in &method.parameters {
            let pann = parameter_annotations(unit, method, &p.name);
            if pann.iter().any(|a| ann_name(a) == "Context") {
                continue;
            }
            let ty = self.resolve(
                unit,
                &parse_type(p.param_type.as_deref().unwrap_or("?")),
                &BTreeMap::new(),
            );
            let mut schema = self.schema(&ty, 0);
            constraints(&mut schema, &pann);
            if let Some(a) = pann.iter().find(|a| ann_name(a) == "RequestBody") {
                request_body = Some(
                    json!({"required":arg(a,"required").as_deref()!=Some("false")&&ty.name!="java.util.Optional","content":media(&anns,"consumes",&schema)}),
                );
                continue;
            }
            let binding = pann.iter().find(|a| {
                matches!(
                    ann_name(a),
                    "PathVariable"
                        | "RequestParam"
                        | "RequestHeader"
                        | "CookieValue"
                        | "PathParam"
                        | "QueryParam"
                        | "HeaderParam"
                        | "CookieParam"
                )
            });
            if let Some(a) = binding {
                let location = match ann_name(a) {
                    "PathVariable" | "PathParam" => "path",
                    "RequestHeader" | "HeaderParam" => "header",
                    "CookieValue" | "CookieParam" => "cookie",
                    _ => "query",
                };
                let default = arg(a, "defaultValue").or_else(|| {
                    pann.iter()
                        .find(|a| ann_name(a) == "DefaultValue")
                        .and_then(|a| arg(a, "value"))
                });
                if let Some(value) = &default {
                    schema["default"] = if schema["type"] == "string" {
                        json!(value)
                    } else {
                        serde_json::from_str(value).unwrap_or(json!(value))
                    };
                }
                wrap_reference(&mut schema);
                let required = location == "path"
                    || (matches!(
                        ann_name(a),
                        "RequestParam" | "RequestHeader" | "CookieValue"
                    ) && arg(a, "required").as_deref() != Some("false")
                        && default.is_none()
                        && ty.name != "java.util.Optional");
                parameters.push(json!({"name":named(a).unwrap_or_else(||p.name.clone()),"in":location,"required":required,"schema":schema}));
            } else if pann
                .iter()
                .any(|a| matches!(ann_name(a), "RequestPart" | "ModelAttribute" | "BeanParam"))
            {
                self.unknown.insert(format!(
                    "unsupported parameter binding: {}.{}",
                    op.symbol.qualified_name, p.name
                ));
            } else if anns.iter().any(|a| {
                matches!(
                    ann_name(a),
                    "Path" | "GET" | "POST" | "PUT" | "DELETE" | "PATCH"
                )
            }) && !matches!(
                ty.name.as_str(),
                "jakarta.ws.rs.core.Context" | "javax.ws.rs.core.Context"
            ) {
                request_body = Some(json!({"content":media(&anns,"consumes",&schema)}));
            }
        }
        if let Some(path) = &op.path {
            for part in path.split('/') {
                if let Some(name) = part.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
                    if !parameters
                        .iter()
                        .any(|p| p["in"] == "path" && p["name"] == name)
                    {
                        parameters.push(json!({"name":name,"in":"path","required":true,"schema":{"type":"string","x-schema-unresolved":true}}));
                    }
                }
            }
        }
        let return_type = method.return_type.as_deref().unwrap_or("?");
        let ty = self.resolve(unit, &parse_type(return_type), &BTreeMap::new());
        let no_body = matches!(ty.name.as_str(), "void" | "java.lang.Void")
            || wrapper(&ty.name) && ty.args.first().is_some_and(|t| t.name == "java.lang.Void");
        let declared_status = anns
            .iter()
            .rev()
            .find(|a| ann_name(a) == "ResponseStatus")
            .and_then(|a| arg(a, "code").or_else(|| arg(a, "value")));
        let status = if let Some(raw) = declared_status {
            status(&raw).unwrap_or_else(|| {
                self.unknown.insert(format!("HTTP response status: {raw}"));
                "default".into()
            })
        } else {
            "200".into()
        };
        let mut response = json!({"description":"Response"});
        if !no_body && status != "204" && status != "304" {
            let schema = self.schema(&ty, 0);
            response["content"] = media(&anns, "produces", &schema);
        }
        let mut operation = json!({"operationId":format!("{}-{}",op.symbol.qualified_name,&super::store::hash(format!("{}:{:?}",op.path.as_deref().unwrap_or(""),op.methods).as_bytes())[..8]),"responses":{status:response}});
        if !parameters.is_empty() {
            operation["parameters"] = json!(parameters);
        }
        if let Some(body) = request_body {
            operation["requestBody"] = body;
        }
        operation
    }
}
fn parameter_annotations<'a>(unit: &'a Unit, method: &Symbol, name: &str) -> Vec<&'a Relation> {
    let (Some(start), Some(end)) = (method.start_byte, method.end_byte) else {
        return vec![];
    };
    let Some(mut node) = unit
        .parsed
        .tree()
        .root_node()
        .descendant_for_byte_range(start as usize, (end as usize).saturating_sub(1))
    else {
        return vec![];
    };
    while !matches!(
        node.kind(),
        "method_declaration" | "constructor_declaration"
    ) {
        let Some(parent) = node.parent() else {
            return vec![];
        };
        node = parent;
    }
    let Some(params) = node.child_by_field_name("parameters") else {
        return vec![];
    };
    let mut cursor = params.walk();
    for parameter in params.named_children(&mut cursor) {
        if parameter
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(&unit.source).ok())
            != Some(name)
        {
            continue;
        }
        return unit
            .extracted
            .relations
            .iter()
            .filter(|r| {
                r.relation_type == RelationType::AnnotatedWith
                    && r.metadata["annotation_scope"] == "Parameter"
                    && r.from == method.qualified_name.as_deref().unwrap_or("")
                    && r.call_site_start_byte.is_some_and(|byte| {
                        (parameter.start_byte()..parameter.end_byte()).contains(&(byte as usize))
                    })
                    && known_annotation(unit, r)
            })
            .collect();
    }
    vec![]
}
fn constraints(schema: &mut Value, anns: &[&Relation]) -> bool {
    let mut required = false;
    for a in anns {
        match ann_name(a) {
            "NotNull" => required = true,
            "NotBlank" | "NotEmpty" => {
                required = true;
                let key = match schema["type"].as_str() {
                    Some("array") => "minItems",
                    Some("object") => "minProperties",
                    _ => "minLength",
                };
                schema[key] = json!(1);
            }
            "Size" => {
                for (arg_name, prefix) in [("min", "min"), ("max", "max")] {
                    if let Some(value) = arg(a, arg_name).and_then(|v| v.parse::<u64>().ok()) {
                        let suffix = match schema["type"].as_str() {
                            Some("array") => "Items",
                            Some("object") => "Properties",
                            _ => "Length",
                        };
                        schema[format!("{prefix}{suffix}")] = json!(value);
                    }
                }
            }
            "Min" | "Max" | "DecimalMin" | "DecimalMax" => {
                if let Some(value) =
                    arg(a, "value").and_then(|v| v.parse::<serde_json::Number>().ok())
                {
                    let lower = ann_name(a).ends_with("Min");
                    schema[if lower { "minimum" } else { "maximum" }] = json!(value);
                    if arg(a, "inclusive").as_deref() == Some("false") {
                        schema[if lower {
                            "exclusiveMinimum"
                        } else {
                            "exclusiveMaximum"
                        }] = json!(true);
                    }
                }
            }
            "Positive" | "PositiveOrZero" | "Negative" | "NegativeOrZero" => {
                let positive = ann_name(a).starts_with("Positive");
                schema[if positive { "minimum" } else { "maximum" }] = json!(0);
                if !ann_name(a).ends_with("OrZero") {
                    schema[if positive {
                        "exclusiveMinimum"
                    } else {
                        "exclusiveMaximum"
                    }] = json!(true);
                }
            }
            "Pattern" => {
                if let Some(v) = arg(a, "regexp") {
                    schema["pattern"] = json!(v);
                }
            }
            "JsonProperty" => {
                required |= arg(a, "required").as_deref() == Some("true");
                if let Some(access) = arg(a, "access") {
                    if access.ends_with("READ_ONLY") {
                        schema["readOnly"] = json!(true);
                    }
                    if access.ends_with("WRITE_ONLY") {
                        schema["writeOnly"] = json!(true);
                    }
                }
            }
            "Schema" => {
                for key in ["description", "format", "example"] {
                    if let Some(value) = arg(a, key) {
                        schema[key] = json!(value);
                    }
                }
            }
            _ => {}
        }
    }
    wrap_reference(schema);
    required
}
fn wrap_reference(schema: &mut Value) {
    // OpenAPI 3.0 ignores siblings of a Reference Object. Keep property
    // constraints/defaults on a surrounding Schema Object instead.
    if schema.get("$ref").is_some() && schema.as_object().is_some_and(|s| s.len() > 1) {
        let reference = schema.as_object_mut().unwrap().remove("$ref").unwrap();
        schema["allOf"] = json!([{"$ref":reference}]);
    }
}
fn media(anns: &[&Relation], key: &str, schema: &Value) -> Value {
    let declaration = anns.iter().rev().find_map(|a| {
        arg(a, key).or_else(|| {
            ((key == "produces" && ann_name(a) == "Produces")
                || (key == "consumes" && ann_name(a) == "Consumes"))
                .then(|| arg(a, "value"))
                .flatten()
        })
    });
    let values = declaration
        .map(|s| {
            if s.starts_with('{') {
                s.trim_matches(['{', '}'])
                    .split(',')
                    .map(|v| v.trim().trim_matches('"').to_owned())
                    .collect::<Vec<_>>()
            } else {
                vec![s]
            }
        })
        .unwrap_or_else(|| vec!["application/json".into()]);
    let mut content = Map::new();
    for value in values {
        let value = match value.rsplit('.').next().unwrap_or(&value) {
            "APPLICATION_JSON" | "APPLICATION_JSON_VALUE" => "application/json".to_owned(),
            "TEXT_PLAIN" | "TEXT_PLAIN_VALUE" => "text/plain".to_owned(),
            "APPLICATION_XML" | "APPLICATION_XML_VALUE" => "application/xml".to_owned(),
            "APPLICATION_OCTET_STREAM" | "APPLICATION_OCTET_STREAM_VALUE" => {
                "application/octet-stream".to_owned()
            }
            _ => value,
        };
        if value.contains('/') && !value.contains("${") {
            content.insert(value, json!({"schema":schema}));
        } else {
            content.insert(
                "*/*".into(),
                json!({"schema":schema,"x-java-media-type":value}),
            );
        }
    }
    json!(content)
}
fn status(raw: &str) -> Option<String> {
    Some(
        match raw.rsplit('.').next()? {
            "OK" => "200",
            "CREATED" => "201",
            "ACCEPTED" => "202",
            "NO_CONTENT" => "204",
            "BAD_REQUEST" => "400",
            "NOT_FOUND" => "404",
            "CONFLICT" => "409",
            "INTERNAL_SERVER_ERROR" => "500",
            value if value.len() == 3 && value.chars().all(|c| c.is_ascii_digit()) => value,
            _ => return None,
        }
        .into(),
    )
}
