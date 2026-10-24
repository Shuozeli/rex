//! Schema registry: turns proto messages annotated with `rex.schema.v1` options into the
//! record/field specs the engine coerces values into (decisions.md U1, U3).

use prost_reflect::{
    DescriptorPool, EnumDescriptor, ExtensionDescriptor, FieldDescriptor, Kind as PbKind, MessageDescriptor, Value,
};
use rex_proto::annotations as ann;

#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    #[error("descriptor pool is missing the rex.schema.v1 annotation extension `{0}`")]
    MissingAnnotation(&'static str),
    #[error("unknown schema message `{0}`")]
    UnknownMessage(String),
    #[error("schema `{message}` field `{field}`: {reason}")]
    InvalidField {
        message: String,
        field: String,
        reason: String,
    },
    #[error("schema `{0}` is not annotated as a record (missing (rex.schema.v1.record).schema_org)")]
    NotARecord(String),
    #[error(transparent)]
    Descriptor(#[from] prost_reflect::DescriptorError),
}

/// Semantic kind of a string-typed field (`rex.schema.v1.ValueKind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StringKind {
    String,
    Text,
    Html,
    Url,
    Decimal,
    Date,
}

/// What a field's value is coerced into.
#[derive(Clone, Debug)]
pub enum Target {
    Str(StringKind),
    I32,
    I64,
    U32,
    U64,
    F32,
    F64,
    Bool,
    Timestamp,
    Enum(EnumDescriptor),
    Message(MessageDescriptor),
}

#[derive(Clone, Debug)]
pub struct FieldSpec {
    pub desc: FieldDescriptor,
    pub name: String,
    pub target: Target,
    pub repeated: bool,
    pub schema_org: String,
    pub required: bool,
    pub max_items: u32,
}

#[derive(Clone, Debug)]
pub struct RecordSpec {
    pub desc: MessageDescriptor,
    pub schema_org: String,
    pub schema_org_also: Vec<String>,
    pub primary: Vec<String>,
    /// Schema.org fallback order (decisions.md U36): higher wins among a page's mapped types.
    pub fallback_rank: i32,
    pub fields: Vec<FieldSpec>,
}

impl RecordSpec {
    pub fn full_name(&self) -> &str {
        self.desc.full_name()
    }

    pub fn field(&self, name: &str) -> Option<&FieldSpec> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// schema.org types this record answers to: the primary type first, then the aliases.
    pub fn schema_org_types(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.schema_org.is_empty() {
            out.push(self.schema_org.clone());
        }
        out.extend(self.schema_org_also.iter().cloned());
        out
    }

    /// The single required string field, used when a scalar is coerced into this record
    /// (decisions.md U7: "Jane Doe" -> Person{name}).
    pub fn scalar_slot(&self) -> Option<&FieldSpec> {
        let mut slots = self
            .fields
            .iter()
            .filter(|f| f.required && !f.repeated && matches!(f.target, Target::Str(_)));
        let first = slots.next()?;
        if slots.next().is_some() {
            return None;
        }
        Some(first)
    }
}

#[derive(Clone)]
pub struct SchemaRegistry {
    pool: DescriptorPool,
    ext_record: ExtensionDescriptor,
    ext_field: ExtensionDescriptor,
    ext_enum_value: ExtensionDescriptor,
}

impl SchemaRegistry {
    pub fn new(pool: DescriptorPool) -> Result<Self, SchemaError> {
        let ext = |name: &'static str| {
            pool.get_extension_by_name(name)
                .ok_or(SchemaError::MissingAnnotation(name))
        };
        Ok(Self {
            ext_record: ext("rex.schema.v1.record")?,
            ext_field: ext("rex.schema.v1.field")?,
            ext_enum_value: ext("rex.schema.v1.enum_value")?,
            pool,
        })
    }

    /// Registry over the descriptor set embedded in `rex-proto`.
    pub fn embedded() -> Result<Self, SchemaError> {
        Self::new(rex_proto::descriptor_pool()?)
    }

    pub fn pool(&self) -> &DescriptorPool {
        &self.pool
    }

    /// Resolves and validates a record message (and, recursively, its nested records).
    pub fn record(&self, full_name: &str) -> Result<RecordSpec, SchemaError> {
        let desc = self
            .pool
            .get_message_by_name(full_name)
            .ok_or_else(|| SchemaError::UnknownMessage(full_name.to_string()))?;
        self.record_of(&desc)
    }

    pub fn record_of(&self, desc: &MessageDescriptor) -> Result<RecordSpec, SchemaError> {
        self.record_checked(desc, &mut Vec::new())
    }

    /// `record_of`, carrying the messages being validated so a self-referencing schema terminates.
    fn record_checked(&self, desc: &MessageDescriptor, visiting: &mut Vec<String>) -> Result<RecordSpec, SchemaError> {
        visiting.push(desc.full_name().to_string());
        let out = self.record_inner(desc, visiting);
        visiting.pop();
        out
    }

    fn record_inner(&self, desc: &MessageDescriptor, visiting: &mut Vec<String>) -> Result<RecordSpec, SchemaError> {
        let opts: ann::RecordOptions = self.read_ext(&desc.options(), &self.ext_record);
        if opts.schema_org.is_empty() {
            return Err(SchemaError::NotARecord(desc.full_name().to_string()));
        }
        let mut fields = Vec::new();
        for f in desc.fields() {
            fields.push(self.field_spec(desc, &f, visiting)?);
        }
        for p in &opts.primary {
            if !fields.iter().any(|f| &f.name == p) {
                return Err(SchemaError::InvalidField {
                    message: desc.full_name().to_string(),
                    field: p.clone(),
                    reason: "listed in `primary` but not a field".into(),
                });
            }
        }
        Ok(RecordSpec {
            desc: desc.clone(),
            schema_org: opts.schema_org,
            schema_org_also: opts.schema_org_also,
            primary: opts.primary,
            fallback_rank: opts.fallback_rank,
            fields,
        })
    }

    /// Every top-level schema in `rex.schema.*` (annotated records outside the annotations package).
    pub fn schemas(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .pool
            .all_messages()
            .filter(|m| m.package_name().starts_with("rex.schema.") && m.package_name() != "rex.schema.v1")
            .filter(|m| m.parent_message().is_none())
            .filter(|m| {
                let o: ann::RecordOptions = self.read_ext(&m.options(), &self.ext_record);
                !o.schema_org.is_empty()
            })
            .map(|m| m.full_name().to_string())
            .collect();
        out.sort();
        out
    }

    /// The schema.org values an enum value answers to, plus its own name.
    pub fn enum_value_by_label(&self, e: &EnumDescriptor, label: &str) -> Option<i32> {
        let want = label.trim();
        if want.is_empty() {
            return None;
        }
        let want_short = want.rsplit('/').next().unwrap_or(want);
        for v in e.values() {
            let opts: ann::EnumValueOptions = self.read_ext(&v.options(), &self.ext_enum_value);
            for s in &opts.schema_org {
                let s_short = s.rsplit('/').next().unwrap_or(s);
                if s.eq_ignore_ascii_case(want) || s_short.eq_ignore_ascii_case(want_short) {
                    return Some(v.number());
                }
            }
        }
        let prefix = enum_prefix(e);
        for v in e.values() {
            let name = v.name();
            let short = name.strip_prefix(prefix.as_str()).unwrap_or(name);
            let norm = |s: &str| s.replace(['-', ' '], "_");
            if name.eq_ignore_ascii_case(want) || short.eq_ignore_ascii_case(&norm(want)) {
                return Some(v.number());
            }
        }
        None
    }

    fn field_spec(
        &self,
        msg: &MessageDescriptor,
        f: &FieldDescriptor,
        visiting: &mut Vec<String>,
    ) -> Result<FieldSpec, SchemaError> {
        let err = |reason: &str| SchemaError::InvalidField {
            message: msg.full_name().to_string(),
            field: f.name().to_string(),
            reason: reason.to_string(),
        };
        if f.is_map() {
            return Err(err("map<> fields are not allowed in schemas (decisions.md U1/U3)"));
        }
        let opts: ann::FieldOptions = self.read_ext(&f.options(), &self.ext_field);
        let kind = ann::ValueKind::try_from(opts.kind).map_err(|_| err("unknown ValueKind"))?;
        let target = match f.kind() {
            PbKind::String => Target::Str(match kind {
                ann::ValueKind::Unspecified | ann::ValueKind::String => StringKind::String,
                ann::ValueKind::Text => StringKind::Text,
                ann::ValueKind::Html => StringKind::Html,
                ann::ValueKind::Url => StringKind::Url,
                ann::ValueKind::Decimal => StringKind::Decimal,
                ann::ValueKind::Date => StringKind::Date,
            }),
            other => {
                if kind != ann::ValueKind::Unspecified {
                    return Err(err("`kind` is only valid on string fields"));
                }
                match other {
                    PbKind::Int32 | PbKind::Sint32 | PbKind::Sfixed32 => Target::I32,
                    PbKind::Int64 | PbKind::Sint64 | PbKind::Sfixed64 => Target::I64,
                    PbKind::Uint32 | PbKind::Fixed32 => Target::U32,
                    PbKind::Uint64 | PbKind::Fixed64 => Target::U64,
                    PbKind::Float => Target::F32,
                    PbKind::Double => Target::F64,
                    PbKind::Bool => Target::Bool,
                    PbKind::Enum(e) => Target::Enum(e),
                    PbKind::Message(m) if m.full_name() == "google.protobuf.Timestamp" => Target::Timestamp,
                    PbKind::Message(m) => {
                        // Nested messages must themselves be records (validated recursively; a message
                        // already being validated up the stack is not re-entered).
                        if !visiting.iter().any(|v| v == m.full_name()) {
                            self.record_checked(&m, visiting)?;
                        }
                        Target::Message(m)
                    }
                    PbKind::Bytes => return Err(err("bytes fields are not supported in schemas")),
                    PbKind::String => unreachable!("handled above"),
                }
            }
        };
        Ok(FieldSpec {
            desc: f.clone(),
            name: f.name().to_string(),
            target,
            repeated: f.is_list(),
            schema_org: opts.schema_org,
            required: opts.required,
            max_items: opts.max_items,
        })
    }

    fn read_ext<T: prost::Message + Default>(
        &self,
        options: &prost_reflect::DynamicMessage,
        ext: &ExtensionDescriptor,
    ) -> T {
        if !options.has_extension(ext) {
            return T::default();
        }
        match options.get_extension(ext).as_ref() {
            Value::Message(m) => m.transcode_to::<T>().unwrap_or_default(),
            _ => T::default(),
        }
    }
}

/// Common `UPPER_SNAKE_` prefix of an enum's values (proto3 style), e.g. "AVAILABILITY_".
fn enum_prefix(e: &EnumDescriptor) -> String {
    let base: String = e
        .name()
        .chars()
        .enumerate()
        .flat_map(|(i, c)| {
            if c.is_uppercase() && i > 0 {
                vec!['_', c]
            } else {
                vec![c.to_ascii_uppercase()]
            }
        })
        .collect();
    format!("{base}_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_self_referencing_record_validates_without_recursing_forever() {
        // Arrange: the embedded descriptors plus `message Node { string name = 1; repeated Node children = 2; }`
        let base = rex_proto::descriptor_pool().expect("pool");
        let set_desc = base
            .get_message_by_name("google.protobuf.FileDescriptorSet")
            .expect("descriptor.proto");
        let file_desc = base
            .get_message_by_name("google.protobuf.FileDescriptorProto")
            .expect("descriptor.proto");
        let mut set = prost_reflect::DynamicMessage::decode(set_desc, rex_proto::FILE_DESCRIPTOR_SET).expect("fds");
        let cycle = serde_json::json!({
            "name": "rex/schema/cycle/v1/cycle.proto",
            "package": "rex.schema.cycle.v1",
            "dependency": ["rex/schema/v1/annotations.proto"],
            "syntax": "proto3",
            "messageType": [{
                "name": "Node",
                "field": [
                    {"name": "name", "number": 1, "label": "LABEL_OPTIONAL", "type": "TYPE_STRING", "jsonName": "name"},
                    {"name": "children", "number": 2, "label": "LABEL_REPEATED", "type": "TYPE_MESSAGE",
                     "typeName": ".rex.schema.cycle.v1.Node", "jsonName": "children"}
                ],
                "options": {"[rex.schema.v1.record]": {"schemaOrg": "Thing"}}
            }]
        });
        let file = prost_reflect::DynamicMessage::deserialize_with_options(
            file_desc,
            cycle,
            &prost_reflect::DeserializeOptions::new(),
        )
        .expect("file descriptor");
        if let Value::List(files) = set.get_field_by_name("file").expect("file").into_owned() {
            let mut files = files;
            files.push(Value::Message(file));
            set.set_field_by_name("file", Value::List(files));
        }
        let bytes = prost::Message::encode_to_vec(&set);
        let reg =
            SchemaRegistry::new(DescriptorPool::decode(bytes.as_slice()).expect("pool decodes")).expect("registry");

        // Act
        let node = reg.record("rex.schema.cycle.v1.Node");

        // Assert
        let node = node.expect("record");
        assert!(node.field("children").is_some());
    }

    #[test]
    fn article_schema_reads_annotations() {
        // Arrange
        let reg = SchemaRegistry::embedded().expect("registry");

        // Act
        let article = reg.record("rex.schema.article.v1.Article").expect("article");

        // Assert
        assert_eq!(article.schema_org, "NewsArticle");
        assert!(article.schema_org_also.contains(&"BlogPosting".to_string()));
        assert_eq!(article.primary, vec!["body_text".to_string()]);
        let title = article.field("title").expect("title");
        assert!(title.required);
        assert_eq!(title.schema_org, "headline");
        assert!(matches!(
            article.field("canonical_url").unwrap().target,
            Target::Str(StringKind::Url)
        ));
        assert!(matches!(
            article.field("published_at").unwrap().target,
            Target::Timestamp
        ));
        assert!(article.field("authors").unwrap().repeated);
    }

    #[test]
    fn person_scalar_slot_is_name() {
        // Arrange
        let reg = SchemaRegistry::embedded().expect("registry");

        // Act
        let person = reg.record("rex.schema.article.v1.Person").expect("person");

        // Assert
        assert_eq!(person.scalar_slot().map(|f| f.name.as_str()), Some("name"));
    }

    #[test]
    fn schemas_lists_top_level_records() {
        // Arrange
        let reg = SchemaRegistry::embedded().expect("registry");

        // Act
        let schemas = reg.schemas();

        // Assert
        assert!(schemas.contains(&"rex.schema.article.v1.Article".to_string()));
        assert!(schemas.contains(&"rex.schema.feed.v1.Feed".to_string()));
    }
}
