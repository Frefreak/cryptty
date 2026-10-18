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
        match self.output_for(key, modifiers)? {
            Output::Text(text) => Some(text),
            Output::Action(_) => None,
        }
    }

    pub fn enters_vim(&self, key: &Key, modifiers: ModifiersState) -> bool {
        matches!(
            self.output_for(key, modifiers),
            Some(Output::Action(Action::VimMode))
        )
    }

    fn output_for(
        &self,
        key: &Key,
        modifiers: ModifiersState,
    ) -> Option<&Output> {
        if let Some(binding) = self.bindings.iter().rev().find(|binding| {
            binding.key == *key && binding.modifiers == modifiers
        }) {
            return Some(&binding.output);
        }
        let custom_vim = self.bindings.iter().any(|binding| {
            matches!(binding.output, Output::Action(Action::VimMode))
        });
        if !custom_vim
            && *key == Key::Character("j".into())
            && modifiers == (ModifiersState::CONTROL | ModifiersState::SHIFT)
        {
            return Some(&Output::Action(Action::VimMode));
        }
        None
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "Definition")]
struct Binding {
    key: Key,
    modifiers: ModifiersState,
    output: Output,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Definition {
    key: String,
    #[serde(default)]
    modifiers: Vec<Modifier>,
    text: Option<String>,
    action: Option<Action>,
}

#[derive(Clone, Debug)]
enum Output {
    Text(String),
    Action(Action),
}

#[derive(Clone, Debug, Deserialize)]
enum Action {
    VimMode,
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
        let output = match (value.text, value.action) {
            (Some(text), None) => Output::Text(text),
            (None, Some(action)) => Output::Action(action),
            _ => {
                return Err(
                    "Binding requires exactly one of text or action".into()
                );
            }
        };
        Ok(Self {
            key,
            modifiers,
            output,
        })
    }
}
