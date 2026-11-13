//! Port of `Component.swift`.

use crate::ical_text::ICalText;

/// One parameter of a property, such as `TZID` or `PARTSTAT`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ICalParameter {
    /// Upper-case parameter name.
    pub name: String,
    /// Unquoted values; the serializer quotes a value that needs it.
    pub values: Vec<String>,
}

impl ICalParameter {
    /// Swift `init(name:values:)`: the name is uppercased.
    pub fn new(name: &str, values: Vec<String>) -> ICalParameter {
        ICalParameter {
            name: name.to_uppercase(),
            values,
        }
    }

    /// Swift `init(_ name:, _ value:)`.
    pub fn single(name: &str, value: &str) -> ICalParameter {
        ICalParameter::new(name, vec![value.to_string()])
    }
}

/// One content line. `value` is the text after the colon exactly as it appears on the wire (TEXT escapes kept), so a
/// property the library does not model is written back unchanged.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ICalProperty {
    /// Upper-case property name.
    pub name: String,
    /// Parameters in file order.
    pub parameters: Vec<ICalParameter>,
    /// The raw value: the text after the first unquoted colon, escapes untouched.
    pub value: String,
}

impl ICalProperty {
    /// Swift `init(name:parameters:value:)`: the name is uppercased.
    pub fn new(
        name: &str,
        parameters: Vec<ICalParameter>,
        value: impl Into<String>,
    ) -> ICalProperty {
        ICalProperty {
            name: name.to_uppercase(),
            parameters,
            value: value.into(),
        }
    }

    /// Swift `init(name:value:)` (no parameters).
    pub fn with_value(name: &str, value: impl Into<String>) -> ICalProperty {
        ICalProperty::new(name, Vec::new(), value)
    }

    /// Swift `init(name:text:parameters:)`: a TEXT property; `text` is escaped for the wire.
    pub fn from_text(name: &str, text: &str, parameters: Vec<ICalParameter>) -> ICalProperty {
        ICalProperty::new(name, parameters, ICalText::escape(text))
    }

    /// The value read as TEXT.
    pub fn text(&self) -> String {
        ICalText::unescape(&self.value)
    }

    /// The first value of the parameter, or `None`.
    pub fn parameter(&self, name: &str) -> Option<&str> {
        let key = name.to_uppercase();
        self.parameters
            .iter()
            .find(|parameter| parameter.name == key)?
            .values
            .first()
            .map(String::as_str)
    }

    /// Sets the parameter to one value, or removes it for `None`.
    pub fn set_parameter(&mut self, name: &str, value: Option<&str>) {
        let key = name.to_uppercase();
        let Some(value) = value else {
            self.parameters.retain(|parameter| parameter.name != key);
            return;
        };
        if let Some(existing) = self
            .parameters
            .iter_mut()
            .find(|parameter| parameter.name == key)
        {
            existing.values = vec![value.to_string()];
        } else {
            self.parameters.push(ICalParameter::single(&key, value));
        }
    }
}

/// A component (`VCALENDAR`, `VEVENT`, `VALARM`, `VTIMEZONE`, ...) with its properties and children, in file order.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ICalComponent {
    /// Upper-case component name.
    pub name: String,
    /// The component's own properties in file order.
    pub properties: Vec<ICalProperty>,
    /// Nested components (for example `VALARM` inside `VEVENT`) in file order.
    pub components: Vec<ICalComponent>,
}

impl ICalComponent {
    /// Swift `init(name:properties:components:)`: the name is uppercased.
    pub fn new(
        name: &str,
        properties: Vec<ICalProperty>,
        components: Vec<ICalComponent>,
    ) -> ICalComponent {
        ICalComponent {
            name: name.to_uppercase(),
            properties,
            components,
        }
    }

    /// The first property with this name (case-insensitive), or `None`.
    pub fn property(&self, name: &str) -> Option<&ICalProperty> {
        let key = name.to_uppercase();
        self.properties.iter().find(|property| property.name == key)
    }

    /// Swift `properties(named:)`.
    pub fn properties_named(&self, name: &str) -> Vec<&ICalProperty> {
        let key = name.to_uppercase();
        self.properties
            .iter()
            .filter(|property| property.name == key)
            .collect()
    }

    /// Swift `components(named:)`.
    pub fn components_named(&self, name: &str) -> Vec<&ICalComponent> {
        let key = name.to_uppercase();
        self.components
            .iter()
            .filter(|component| component.name == key)
            .collect()
    }

    /// Replaces every property with this name by `property`, at the position of the first one (or appends it).
    pub fn set(&mut self, property: ICalProperty) {
        let Some(first) = self
            .properties
            .iter()
            .position(|existing| existing.name == property.name)
        else {
            self.properties.push(property);
            return;
        };
        let name = property.name.clone();
        self.properties[first] = property;
        let mut index = first + 1;
        while index < self.properties.len() {
            if self.properties[index].name == name {
                self.properties.remove(index);
            } else {
                index += 1;
            }
        }
    }

    /// Sets a TEXT property, or removes it for `None` or empty text.
    pub fn set_text(&mut self, name: &str, text: Option<&str>) {
        match text {
            Some(text) if !text.is_empty() => {
                self.set(ICalProperty::from_text(name, text, Vec::new()))
            }
            _ => self.remove_properties_named(name),
        }
    }

    /// Removes every property with this name (case-insensitive).
    pub fn remove_properties_named(&mut self, name: &str) {
        let key = name.to_uppercase();
        self.properties.retain(|property| property.name != key);
    }

    /// Adds `property` after the existing ones, without checking for another of the same name.
    pub fn append(&mut self, property: ICalProperty) {
        self.properties.push(property);
    }
}
