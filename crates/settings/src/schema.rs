//! Stable section schema construction, independent of policy and Rust module placement.
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::Serialize;

/// Ordered object properties and defaults for one typed settings section.
pub struct SectionSchema<'a> {
    schema: Schema,
    generator: &'a mut SchemaGenerator,
}
impl<'a> SectionSchema<'a> {
    pub fn new(generator: &'a mut SchemaGenerator) -> Self {
        Self {
            schema: json_schema!({"type": "object", "additionalProperties": false}),
            generator,
        }
    }
    pub fn field<T: JsonSchema + Serialize>(&mut self, name: &str, default: &T, description: &str) {
        self.property(
            name,
            self.generator.contract().is_deserialize(),
            default,
            description,
            false,
        );
    }
    pub fn optional<T: JsonSchema + Serialize>(
        &mut self,
        name: &str,
        default: &T,
        description: &str,
        skip_default: bool,
    ) {
        self.property(name, true, default, description, skip_default);
    }
    fn property<T: JsonSchema + Serialize>(
        &mut self,
        name: &str,
        optional: bool,
        default: &T,
        description: &str,
        skip_default: bool,
    ) {
        let mut field = self.generator.subschema_for::<T>();
        if !description.is_empty() {
            field.insert("description".into(), description.into());
        }
        if !skip_default {
            field.insert(
                "default".into(),
                serde_json::to_value(default).expect("settings defaults serialize"),
            );
        }
        let object = self.schema.ensure_object();
        let properties = object
            .entry("properties")
            .or_insert_with(|| serde_json::json!({}))
            .as_object_mut()
            .expect("schema properties are an object");
        properties.insert(name.into(), field.to_value());
        if !optional {
            object
                .entry("required")
                .or_insert_with(|| serde_json::json!([]))
                .as_array_mut()
                .expect("schema required is an array")
                .push(name.into());
        }
    }
    pub fn range<T: Serialize>(&mut self, name: &str, minimum: T, maximum: T) {
        let property = self
            .schema
            .ensure_object()
            .get_mut("properties")
            .and_then(|v| v.get_mut(name))
            .and_then(|v| v.as_object_mut())
            .expect("range follows a schema field");
        property.insert(
            "minimum".into(),
            serde_json::to_value(minimum).expect("schema bound serializes"),
        );
        property.insert(
            "maximum".into(),
            serde_json::to_value(maximum).expect("schema bound serializes"),
        );
    }
    pub fn finish(mut self, description: &str) -> Schema {
        self.schema.insert("description".into(), description.into());
        self.schema
    }
}
