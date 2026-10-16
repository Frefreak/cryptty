use serde::Deserialize;
use serde::de::IntoDeserializer;
use winit::keyboard::{Key, ModifiersState, NamedKey};

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Keyboard {
    bindings: Vec<Binding>,
}

impl Keyboard {
    pub fn text_for(
        &self,
        key: &Key,
        modifiers: ModifiersState,
    ) -> Option<&str> {
        self.bindings.iter().rev().find_map(|binding| {
            (binding.key == *key && binding.modifiers == modifiers)
                .then_some(binding.text.as_str())
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "Definition")]
struct Binding {
    key: Key,
    modifiers: ModifiersState,
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Definition {
    key: String,
    #[serde(default)]
    modifiers: Vec<Modifier>,
    text: String,
}

#[derive(Deserialize)]
enum Modifier {
    Shift,
    Control,
    Alt,
    Super,
}

impl TryFrom<Definition> for Binding {
    type Error = String;

    fn try_from(value: Definition) -> Result<Self, Self::Error> {
        let key = if value.key.chars().count() == 1 {
            Key::Character(value.key.into())
        } else {
            let named: Result<NamedKey, serde::de::value::Error> =
                NamedKey::deserialize(value.key.clone().into_deserializer());
            Key::Named(named.map_err(|_| {
                format!("Unknown binding key {:?}; use a named key or one character",
                    value.key)
            })?)
        };
        let modifiers = value.modifiers.into_iter().fold(
            ModifiersState::empty(),
            |flags, modifier| {
                flags
                    | match modifier {
                        Modifier::Shift => ModifiersState::SHIFT,
                        Modifier::Control => ModifiersState::CONTROL,
                        Modifier::Alt => ModifiersState::ALT,
                        Modifier::Super => ModifiersState::SUPER,
                    }
            },
        );
        Ok(Self {
            key,
            modifiers,
            text: value.text,
        })
    }
}
