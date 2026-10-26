use crate::data_component_impl::DataComponentImpl;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::text::TextComponent;

/// Reads a filterable text, which vanilla stores either as a plain string or as
/// a compound with a `raw` field, e.g. `{raw: "A Partner"}`. The raw value is a
/// text component of its own, so it may also be a styled compound such as
/// `{raw: {text: "A Partner", color: "red"}}`.
fn read_component(tag: &NbtTag) -> Option<TextComponent> {
    match tag {
        NbtTag::String(value) => Some(TextComponent::text(value.to_string())),
        NbtTag::Compound(compound) => {
            let component = compound.get("raw").unwrap_or(tag);
            Some(TextComponent::from_nbt(component))
        }
        _ => None,
    }
}

/// Reads a filterable text whose content is a plain string, like writable book
/// pages and written book titles in vanilla.
fn read_text(tag: &NbtTag) -> Option<String> {
    read_component(tag).map(TextComponent::get_text)
}

fn text_tag(value: &str) -> NbtTag {
    text_component_tag(&TextComponent::text(value.to_string()))
}

fn text_component_tag(component: &TextComponent) -> NbtTag {
    let mut compound = NbtCompound::new();
    compound.put("raw", NbtTag::Compound(component.0.to_nbt_compound()));
    NbtTag::Compound(compound)
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct WritableBookContentImpl {
    pub pages: Vec<String>,
}
impl WritableBookContentImpl {
    pub fn read_data(tag: &NbtTag) -> Option<Self> {
        let mut pages = Vec::new();
        if let NbtTag::Compound(c) = tag
            && let Some(NbtTag::List(l)) = c.get("pages")
        {
            for item in l {
                if let Some(page) = read_text(item) {
                    pages.push(page);
                }
            }
        }
        Some(Self { pages })
    }
}
impl DataComponentImpl for WritableBookContentImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        let pages_tags: Vec<NbtTag> = self.pages.iter().map(|p| text_tag(p)).collect();
        compound.put("pages", NbtTag::List(pages_tags));
        NbtTag::Compound(compound)
    }
    default_impl!(WritableBookContent);
}

/// Vanilla `WrittenBookContent` stores pages as filterable components, so the
/// parsed component is kept instead of being flattened to plain text.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct WrittenBookContentImpl {
    pub title: String,
    pub author: String,
    pub pages: Vec<TextComponent>,
}
impl WrittenBookContentImpl {
    pub fn read_data(tag: &NbtTag) -> Option<Self> {
        let mut pages = Vec::new();
        let mut title = String::new();
        let mut author = String::new();
        if let NbtTag::Compound(c) = tag {
            if let Some(value) = c.get("title").and_then(read_text) {
                title = value;
            }
            if let Some(s) = c.get_string("author") {
                author = s.to_string();
            }
            if let Some(NbtTag::List(l)) = c.get("pages") {
                for item in l {
                    if let Some(page) = read_component(item) {
                        pages.push(page);
                    }
                }
            }
        }
        Some(Self {
            title,
            author,
            pages,
        })
    }
}
impl DataComponentImpl for WrittenBookContentImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        compound.put("title", text_tag(&self.title));
        compound.put_string("author", self.author.clone());
        let pages_tags: Vec<NbtTag> = self.pages.iter().map(text_component_tag).collect();
        compound.put("pages", NbtTag::List(pages_tags));
        NbtTag::Compound(compound)
    }
    default_impl!(WrittenBookContent);
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, Default)]
pub struct DebugStickStateImpl {
    pub properties: Vec<(String, String)>,
}
#[allow(non_upper_case_globals)]
pub const DebugStickStateImpl: DebugStickStateImpl = DebugStickStateImpl {
    properties: Vec::new(),
};
impl DebugStickStateImpl {
    pub fn read_data(data: &NbtTag) -> Option<Self> {
        let mut properties = Vec::new();
        if let Some(compound) = data.extract_compound() {
            for (k, v) in &compound.child_tags {
                if let Some(s) = v.extract_string() {
                    properties.push((k.to_string(), s.to_string()));
                }
            }
        }
        Some(Self { properties })
    }
}
impl DataComponentImpl for DebugStickStateImpl {
    fn write_data(&self) -> NbtTag {
        let mut compound = NbtCompound::new();
        for (k, v) in &self.properties {
            compound.put_string(k.as_str(), v.clone());
        }
        NbtTag::Compound(compound)
    }
    default_impl!(DebugStickState);
}

