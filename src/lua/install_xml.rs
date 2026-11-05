use std::{
    collections::{HashMap, HashSet},
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum InstallXmlError {
    #[error("XML parse error at byte {offset}: {message}")]
    ParseError { offset: usize, message: String },
    #[error("missing root <installer> or <install> element")]
    MissingRoot,
    #[error("invalid module definition: {0}")]
    InvalidModule(String),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstallModuleEntry {
    pub id: String,
    pub enabled: bool,
    pub version: String,
    pub name: String,
    pub description: String,
    pub folder: PathBuf,
    pub entry: PathBuf,
    pub config: Option<PathBuf>,
    pub author: Option<String>,
    pub category: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstallManifest {
    pub version: String,
    pub bot_name: Option<String>,
    pub min_engine_version: Option<String>,
    pub modules: Vec<InstallModuleEntry>,
}

fn validate_relative_path(path: &Path, field: &str) -> Result<(), InstallXmlError> {
    if path.as_os_str().is_empty()
        || !path
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(InstallXmlError::InvalidModule(format!(
            "<module> {field} must be a relative path without parent traversal or absolute roots, got '{}'",
            path.display()
        )));
    }
    Ok(())
}

impl InstallManifest {
    pub fn parse(xml: &str) -> Result<Self, InstallXmlError> {
        let root = parse_xml_to_dom(xml)?;

        if root.tag != "installer" && root.tag != "install" {
            return Err(InstallXmlError::MissingRoot);
        }

        let version = root
            .attributes
            .get("version")
            .cloned()
            .or_else(|| {
                root.find_child("version")
                    .map(|c| c.text.trim().to_string())
            })
            .unwrap_or_else(|| "1.0.0".to_string());

        let bot_name = root.find_child("bot").and_then(|b| {
            b.attributes.get("name").cloned().or_else(|| {
                let t = b.text.trim();
                if t.is_empty() {
                    None
                } else {
                    Some(t.to_string())
                }
            })
        });

        let min_engine_version = root
            .find_child("bot")
            .and_then(|b| b.attributes.get("min_engine_version").cloned())
            .or_else(|| {
                root.find_child("min_engine_version")
                    .map(|c| c.text.trim().to_string())
            });

        let modules_node = root.find_child("modules").unwrap_or(&root);
        let mut modules = Vec::new();
        let mut seen_ids = HashSet::new();

        for child in &modules_node.children {
            if child.tag != "module" && child.tag != "plugin" && child.tag != "feature" {
                continue;
            }

            let id = child
                .attributes
                .get("id")
                .cloned()
                .or_else(|| child.find_child("id").map(|c| c.text.trim().to_string()))
                .ok_or_else(|| {
                    InstallXmlError::InvalidModule("<module> is missing 'id'".to_string())
                })?;

            if id.is_empty() {
                return Err(InstallXmlError::InvalidModule(
                    "<module> id cannot be empty".to_string(),
                ));
            }

            if !seen_ids.insert(id.clone()) {
                return Err(InstallXmlError::InvalidModule(format!(
                    "duplicate module id '{}' in manifest",
                    id
                )));
            }

            let enabled = child
                .attributes
                .get("enabled")
                .map(|s| parse_bool(s))
                .or_else(|| child.find_child("enabled").map(|c| parse_bool(&c.text)))
                .unwrap_or(true);

            let mod_version = child
                .attributes
                .get("version")
                .cloned()
                .or_else(|| {
                    child
                        .find_child("version")
                        .map(|c| c.text.trim().to_string())
                })
                .unwrap_or_else(|| "1.0.0".to_string());

            let name = child
                .find_child("name")
                .map(|c| c.text.trim().to_string())
                .or_else(|| child.attributes.get("name").cloned())
                .unwrap_or_else(|| id.clone());

            let description = child
                .find_child("description")
                .map(|c| c.text.trim().to_string())
                .or_else(|| child.attributes.get("description").cloned())
                .unwrap_or_default();

            let folder_str = child
                .find_child("folder")
                .map(|c| c.text.trim().to_string())
                .or_else(|| child.attributes.get("folder").cloned())
                .unwrap_or_else(|| id.clone());

            let folder_path = PathBuf::from(folder_str);
            validate_relative_path(&folder_path, "folder")?;

            let entry_str = child
                .find_child("entry")
                .map(|c| c.text.trim().to_string())
                .or_else(|| child.attributes.get("entry").cloned())
                .unwrap_or_else(|| "module.lua".to_string());

            let entry_path = PathBuf::from(entry_str);
            validate_relative_path(&entry_path, "entry")?;

            let config_str = child
                .find_child("config")
                .map(|c| c.text.trim().to_string())
                .or_else(|| child.attributes.get("config").cloned())
                .unwrap_or_else(|| "config.json".to_string());

            let config_path = if config_str.is_empty() {
                None
            } else {
                let p = PathBuf::from(config_str);
                validate_relative_path(&p, "config")?;
                Some(p)
            };

            let author = child
                .find_child("author")
                .map(|c| c.text.trim().to_string())
                .or_else(|| child.attributes.get("author").cloned());

            let category = child
                .find_child("category")
                .map(|c| c.text.trim().to_string())
                .or_else(|| child.attributes.get("category").cloned());

            modules.push(InstallModuleEntry {
                id,
                enabled,
                version: mod_version,
                name,
                description,
                folder: folder_path,
                entry: entry_path,
                config: config_path,
                author,
                category,
            });
        }

        Ok(Self {
            version,
            bot_name,
            min_engine_version,
            modules,
        })
    }

    pub fn is_module_enabled(&self, module_id: &str) -> bool {
        self.modules
            .iter()
            .find(|m| m.id == module_id)
            .map(|m| m.enabled)
            .unwrap_or(true)
    }

    pub fn get_module(&self, module_id: &str) -> Option<&InstallModuleEntry> {
        self.modules.iter().find(|m| m.id == module_id)
    }
}

fn parse_bool(val: &str) -> bool {
    let s = val.trim().to_lowercase();
    matches!(s.as_str(), "true" | "1" | "yes" | "on" | "enabled")
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct XmlElement {
    pub tag: String,
    pub attributes: HashMap<String, String>,
    pub text: String,
    pub children: Vec<XmlElement>,
}

impl XmlElement {
    pub fn find_child(&self, tag: &str) -> Option<&XmlElement> {
        self.children.iter().find(|c| c.tag == tag)
    }
}

pub fn parse_xml_to_dom(source: &str) -> Result<XmlElement, InstallXmlError> {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    let mut stack: Vec<XmlElement> = Vec::new();
    let mut root: Option<XmlElement> = None;

    while i < len {
        let prev_main_i = i;
        // Skip whitespace
        while i < len && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= len {
            break;
        }

        if bytes[i] == b'<' {
            // Check for XML declaration <? ... ?>
            if i + 1 < len && bytes[i + 1] == b'?' {
                let start_decl = i;
                let rest = &source[i + 2..];
                let relative_end = rest.find("?>").ok_or_else(|| InstallXmlError::ParseError {
                    offset: start_decl,
                    message: "unclosed XML declaration".to_owned(),
                })?;
                i = i + 2 + relative_end + 2;
                continue;
            }

            // Check for comments <!-- ... -->
            if i + 3 < len && &bytes[i..i + 4] == b"<!--" {
                let start_comment = i;
                let rest = &source[i + 4..];
                let relative_end = rest
                    .find("-->")
                    .ok_or_else(|| InstallXmlError::ParseError {
                        offset: start_comment,
                        message: "unclosed comment".to_owned(),
                    })?;
                i = i + 4 + relative_end + 3;
                continue;
            }

            // Check for CDATA <![CDATA[ ... ]]>
            if i + 8 < len && &bytes[i..i + 9] == b"<![CDATA[" {
                let start_cdata = i + 9;
                let rest = &source[start_cdata..];
                let relative_end = rest
                    .find("]]>")
                    .ok_or_else(|| InstallXmlError::ParseError {
                        offset: start_cdata,
                        message: "unclosed CDATA section".to_owned(),
                    })?;
                let end_cdata = start_cdata + relative_end;
                let cdata_text = &source[start_cdata..end_cdata];
                if let Some(curr) = stack.last_mut() {
                    curr.text.push_str(cdata_text);
                }
                i = end_cdata + 3;
                continue;
            }

            // Check for closing tag </tag>
            if i + 1 < len && bytes[i + 1] == b'/' {
                i += 2;
                let start_tag = i;
                while i < len && bytes[i] != b'>' && !bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                let tag_name = &source[start_tag..i];
                while i < len && bytes[i] != b'>' {
                    i += 1;
                }
                if i < len && bytes[i] == b'>' {
                    i += 1;
                } else {
                    return Err(InstallXmlError::ParseError {
                        offset: start_tag,
                        message: format!("unclosed closing tag </{tag_name}>"),
                    });
                }

                let popped = stack.pop().ok_or_else(|| InstallXmlError::ParseError {
                    offset: start_tag,
                    message: format!("unexpected closing tag </{tag_name}>"),
                })?;

                if popped.tag != tag_name {
                    return Err(InstallXmlError::ParseError {
                        offset: start_tag,
                        message: format!(
                            "mismatched closing tag: expected </{}>, found </{tag_name}>",
                            popped.tag
                        ),
                    });
                }

                if let Some(parent) = stack.last_mut() {
                    parent.children.push(popped);
                } else if root.is_none() {
                    root = Some(popped);
                }
                continue;
            }

            // Opening tag <tag attr="val"> or <tag attr="val" />
            i += 1; // skip '<'
            let start_tag = i;
            while i < len && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' && bytes[i] != b'/'
            {
                i += 1;
            }
            let tag_name = source[start_tag..i].trim().to_string();
            if tag_name.is_empty() {
                return Err(InstallXmlError::ParseError {
                    offset: start_tag,
                    message: "empty tag name".to_string(),
                });
            }

            let mut attributes = HashMap::new();
            let mut is_self_closing = false;
            let mut tag_closed = false;

            while i < len {
                let prev_attr_i = i;

                while i < len && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                if i >= len {
                    break;
                }

                if bytes[i] == b'/' && i + 1 < len && bytes[i + 1] == b'>' {
                    is_self_closing = true;
                    tag_closed = true;
                    i += 2;
                    break;
                }

                if bytes[i] == b'>' {
                    tag_closed = true;
                    i += 1;
                    break;
                }

                if bytes[i] == b'/' {
                    return Err(InstallXmlError::ParseError {
                        offset: i,
                        message: "unexpected '/' in attribute list (expected '/>')".to_owned(),
                    });
                }

                // Parse attribute name
                let attr_start = i;
                while i < len
                    && bytes[i] != b'='
                    && bytes[i] != b'>'
                    && bytes[i] != b'/'
                    && !bytes[i].is_ascii_whitespace()
                {
                    i += 1;
                }
                let attr_name = source[attr_start..i].trim().to_string();
                if attr_name.is_empty() {
                    return Err(InstallXmlError::ParseError {
                        offset: attr_start,
                        message: "expected XML attribute name".to_owned(),
                    });
                }

                while i < len && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }

                let attr_val = if i < len && bytes[i] == b'=' {
                    i += 1; // skip '='
                    while i < len && bytes[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    if i < len && (bytes[i] == b'"' || bytes[i] == b'\'') {
                        let quote = bytes[i];
                        i += 1;
                        let val_start = i;
                        while i < len && bytes[i] != quote {
                            i += 1;
                        }
                        let val = &source[val_start..i];
                        if i < len && bytes[i] == quote {
                            i += 1;
                        }
                        unescape_xml(val)
                    } else {
                        let val_start = i;
                        while i < len && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                            i += 1;
                        }
                        unescape_xml(&source[val_start..i])
                    }
                } else {
                    String::new()
                };

                attributes.insert(attr_name, attr_val);

                if i <= prev_attr_i {
                    return Err(InstallXmlError::ParseError {
                        offset: i,
                        message: "parser made no forward progress in attribute list".to_owned(),
                    });
                }
            }

            if !tag_closed {
                return Err(InstallXmlError::ParseError {
                    offset: start_tag,
                    message: "unclosed opening tag".to_owned(),
                });
            }

            let elem = XmlElement {
                tag: tag_name,
                attributes,
                text: String::new(),
                children: Vec::new(),
            };

            if is_self_closing {
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(elem);
                } else if root.is_none() {
                    root = Some(elem);
                }
            } else {
                stack.push(elem);
            }
        } else {
            // Text node
            let text_start = i;
            while i < len && bytes[i] != b'<' {
                i += 1;
            }
            let raw_text = &source[text_start..i];
            let unescaped = unescape_xml(raw_text);
            if let Some(curr) = stack.last_mut() {
                curr.text.push_str(&unescaped);
            }
        }

        if i <= prev_main_i {
            return Err(InstallXmlError::ParseError {
                offset: i,
                message: "parser made no forward progress".to_owned(),
            });
        }
    }

    if let Some(r) = root {
        Ok(r)
    } else if stack.len() == 1 {
        Ok(stack.pop().unwrap())
    } else {
        Err(InstallXmlError::MissingRoot)
    }
}

fn unescape_xml(input: &str) -> String {
    input
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_installer_xml_with_attributes_and_tags() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- Installer Manifest -->
<installer version="2.5.0">
    <bot name="rusty-discord-bot" min_engine_version="0.2.0" />
    <modules>
        <module id="core" enabled="true" version="1.0.0">
            <name>Core System</name>
            <description>Podstawowe funkcje</description>
            <folder>core</folder>
            <entry>module.lua</entry>
            <config>config.json</config>
            <author>Adrian Banach</author>
            <category>platform</category>
        </module>
        <module id="economy" enabled="false" version="1.2.0">
            <name>Economy System</name>
            <description>Wirtualna ekonomia</description>
            <folder>economy</folder>
            <entry>module.lua</entry>
            <config>config.json</config>
            <author>Adrian Banach</author>
            <category>economy</category>
        </module>
    </modules>
</installer>"#;

        let manifest = InstallManifest::parse(xml).expect("manifest should parse");
        assert_eq!(manifest.version, "2.5.0");
        assert_eq!(manifest.bot_name.as_deref(), Some("rusty-discord-bot"));
        assert_eq!(manifest.min_engine_version.as_deref(), Some("0.2.0"));
        assert_eq!(manifest.modules.len(), 2);

        let core = &manifest.modules[0];
        assert_eq!(core.id, "core");
        assert!(core.enabled);
        assert_eq!(core.version, "1.0.0");
        assert_eq!(core.name, "Core System");
        assert_eq!(core.folder, PathBuf::from("core"));
        assert_eq!(core.entry, PathBuf::from("module.lua"));
        assert_eq!(core.config, Some(PathBuf::from("config.json")));
        assert_eq!(core.author.as_deref(), Some("Adrian Banach"));
        assert_eq!(core.category.as_deref(), Some("platform"));

        let economy = &manifest.modules[1];
        assert_eq!(economy.id, "economy");
        assert!(!economy.enabled);
        assert_eq!(economy.version, "1.2.0");

        assert!(manifest.is_module_enabled("core"));
        assert!(!manifest.is_module_enabled("economy"));
        assert!(manifest.is_module_enabled("unknown_module"));
    }

    #[test]
    fn parses_tag_heavy_install_xml() {
        let xml = r#"
        <install>
            <version>1.1.0</version>
            <modules>
                <module>
                    <id>tickets</id>
                    <enabled>1</enabled>
                    <version>2.0.0</version>
                    <name>Tickets</name>
                    <description>System ticketów wsparcia</description>
                    <folder>tickets</folder>
                    <entry>module.lua</entry>
                </module>
            </modules>
        </install>
        "#;

        let manifest = InstallManifest::parse(xml).expect("manifest should parse");
        assert_eq!(manifest.version, "1.1.0");
        assert_eq!(manifest.modules.len(), 1);
        assert_eq!(manifest.modules[0].id, "tickets");
        assert!(manifest.modules[0].enabled);
        assert_eq!(manifest.modules[0].version, "2.0.0");
    }

    #[test]
    fn rejects_missing_id() {
        let xml = r#"<installer><modules><module enabled="true" /></modules></installer>"#;
        assert!(InstallManifest::parse(xml).is_err());
    }

    #[test]
    fn rejects_duplicate_module_id() {
        let xml = r#"
        <installer version="1.0.0">
            <modules>
                <module id="demo" enabled="true" />
                <module id="demo" enabled="false" />
            </modules>
        </installer>
        "#;
        let res = InstallManifest::parse(xml);
        assert!(
            matches!(res, Err(InstallXmlError::InvalidModule(msg)) if msg.contains("duplicate module id 'demo'"))
        );
    }

    #[test]
    fn rejects_path_traversal_in_folder_entry_config() {
        let bad_folder = r#"<installer><modules><module id="m1"><folder>../escape</folder></module></modules></installer>"#;
        assert!(
            matches!(InstallManifest::parse(bad_folder), Err(InstallXmlError::InvalidModule(msg)) if msg.contains("folder"))
        );

        let bad_entry = r#"<installer><modules><module id="m2"><folder>pkg</folder><entry>../../evil.lua</entry></module></modules></installer>"#;
        assert!(
            matches!(InstallManifest::parse(bad_entry), Err(InstallXmlError::InvalidModule(msg)) if msg.contains("entry"))
        );

        let bad_config = r#"<installer><modules><module id="m3"><folder>pkg</folder><config>../../private/secret.json</config></module></modules></installer>"#;
        assert!(
            matches!(InstallManifest::parse(bad_config), Err(InstallXmlError::InvalidModule(msg)) if msg.contains("config"))
        );

        let absolute_config = r#"<installer><modules><module id="m4"><folder>pkg</folder><config>/etc/passwd</config></module></modules></installer>"#;
        assert!(
            matches!(InstallManifest::parse(absolute_config), Err(InstallXmlError::InvalidModule(msg)) if msg.contains("config"))
        );
    }

    #[test]
    fn malformed_attribute_slash_must_not_hang_installer() {
        assert!(parse_xml_to_dom("<installer /broken>").is_err());
        assert!(parse_xml_to_dom("<installer/ broken>").is_err());
        assert!(parse_xml_to_dom("<installer/>").is_ok());
        assert!(parse_xml_to_dom("<installer =broken>").is_err());
        assert!(parse_xml_to_dom("<installer").is_err());
    }

    #[test]
    fn test_malformed_xml_fuzz_fragments_never_hang() {
        let bad_inputs = [
            "<installer /",
            "<installer //",
            "<installer ///>",
            "<installer /=",
            "<installer /\"",
            "<installer /'>",
            "<installer =",
            "<installer =/",
            "<installer =>",
            "<installer \"foo\">",
            "<installer 'foo'>",
            "<installer / >",
            "</>",
            "<!",
            "<?",
            "<![CDATA[",
            "<!--",
        ];

        for input in bad_inputs {
            let res = parse_xml_to_dom(input);
            // Must either parse or return Err, but never hang
            let _ = res;
        }
    }

    #[test]
    fn unterminated_cdata_is_parse_error_not_panic() {
        for xml in ["<installer><![CDATA[éa", "<installer><![CDATA[abc"] {
            let result = std::panic::catch_unwind(|| InstallManifest::parse(xml));
            assert!(
                matches!(result, Ok(Err(InstallXmlError::ParseError { .. }))),
                "must reject unclosed CDATA, never panic or silently accept: {:?}",
                xml
            );
        }
        assert!(InstallManifest::parse("<installer><![CDATA[Zażółć]]></installer>").is_ok());
    }

    #[test]
    fn unterminated_declaration_and_comment_are_parse_errors() {
        assert!(parse_xml_to_dom("<?xml version=\"1.0\"").is_err());
        assert!(parse_xml_to_dom("<!-- unclosed comment").is_err());
        assert!(parse_xml_to_dom("<installer></unclosed").is_err());
    }
}
