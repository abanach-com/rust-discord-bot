use std::{
    collections::{HashMap, HashSet},
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAX_INSTALL_XML_BYTES: usize = 512 * 1024;
pub const MAX_XML_DEPTH: usize = 64;
pub const MAX_XML_ELEMENTS: usize = 4096;
pub const MAX_XML_ELEMENT_TEXT_BYTES: usize = 65_536;

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
        if xml.len() > MAX_INSTALL_XML_BYTES {
            return Err(InstallXmlError::ParseError {
                offset: 0,
                message: format!(
                    "install.xml size exceeds maximum limit of {} bytes (got {})",
                    MAX_INSTALL_XML_BYTES,
                    xml.len()
                ),
            });
        }

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
    if source.len() > MAX_INSTALL_XML_BYTES {
        return Err(InstallXmlError::ParseError {
            offset: 0,
            message: format!(
                "install.xml size exceeds maximum limit of {} bytes (got {})",
                MAX_INSTALL_XML_BYTES,
                source.len()
            ),
        });
    }

    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    let mut stack: Vec<XmlElement> = Vec::new();
    let mut root: Option<XmlElement> = None;
    let mut total_elements: usize = 0;

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
                    if curr.text.len().saturating_add(cdata_text.len()) > MAX_XML_ELEMENT_TEXT_BYTES
                    {
                        return Err(InstallXmlError::ParseError {
                            offset: start_cdata,
                            message: format!(
                                "element text content exceeds maximum limit of {MAX_XML_ELEMENT_TEXT_BYTES} bytes"
                            ),
                        });
                    }
                    curr.text.push_str(cdata_text);
                } else if !cdata_text.trim().is_empty() {
                    return Err(InstallXmlError::ParseError {
                        offset: start_cdata,
                        message: "CDATA section outside root element".to_owned(),
                    });
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
                } else {
                    return Err(InstallXmlError::ParseError {
                        offset: start_tag,
                        message: "multiple root elements".to_owned(),
                    });
                }
                continue;
            }

            // Opening tag <tag attr="val"> or <tag attr="val" />
            let start_tag = i;
            if stack.is_empty() && root.is_some() {
                return Err(InstallXmlError::ParseError {
                    offset: start_tag,
                    message: "multiple root elements".to_owned(),
                });
            }

            total_elements = total_elements.saturating_add(1);
            if total_elements > MAX_XML_ELEMENTS {
                return Err(InstallXmlError::ParseError {
                    offset: start_tag,
                    message: format!(
                        "XML element count exceeded maximum limit of {MAX_XML_ELEMENTS}"
                    ),
                });
            }

            if stack.len() >= MAX_XML_DEPTH {
                return Err(InstallXmlError::ParseError {
                    offset: start_tag,
                    message: format!("XML nesting depth exceeded maximum limit of {MAX_XML_DEPTH}"),
                });
            }

            i += 1; // skip '<'
            let start_tag_name = i;
            while i < len && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' && bytes[i] != b'/'
            {
                i += 1;
            }
            let tag_name = source[start_tag_name..i].trim().to_string();
            if tag_name.is_empty() {
                return Err(InstallXmlError::ParseError {
                    offset: start_tag_name,
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

                if attributes.insert(attr_name.clone(), attr_val).is_some() {
                    return Err(InstallXmlError::ParseError {
                        offset: attr_start,
                        message: format!("duplicate XML attribute '{attr_name}'"),
                    });
                }

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
                } else {
                    return Err(InstallXmlError::ParseError {
                        offset: start_tag,
                        message: "multiple root elements".to_owned(),
                    });
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
                if curr.text.len().saturating_add(unescaped.len()) > MAX_XML_ELEMENT_TEXT_BYTES {
                    return Err(InstallXmlError::ParseError {
                        offset: text_start,
                        message: format!(
                            "element text content exceeds maximum limit of {MAX_XML_ELEMENT_TEXT_BYTES} bytes"
                        ),
                    });
                }
                curr.text.push_str(&unescaped);
            } else if !raw_text.trim().is_empty() {
                return Err(InstallXmlError::ParseError {
                    offset: text_start,
                    message: "text content outside root element".to_owned(),
                });
            }
        }

        if i <= prev_main_i {
            return Err(InstallXmlError::ParseError {
                offset: i,
                message: "parser made no forward progress".to_owned(),
            });
        }
    }

    if !stack.is_empty() {
        return Err(InstallXmlError::ParseError {
            offset: len,
            message: "unclosed XML element".to_owned(),
        });
    }

    root.ok_or(InstallXmlError::MissingRoot)
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

    #[test]
    fn rejects_multiple_roots_and_unclosed_fragments() {
        let fixtures = [
            "<installer version=\"1.0.0\"/><other/>",
            "<installer/><broken>",
            "<installer/>unexpected-text",
            "<installer></installer><installer></installer>",
            "unexpected-text<installer/>",
            "<installer><unclosed>",
            "<installer>",
            "<installer><![CDATA[outside]]></installer><![CDATA[invalid]]>",
        ];

        for xml in fixtures {
            let res = InstallManifest::parse(xml);
            assert!(
                matches!(res, Err(InstallXmlError::ParseError { .. })),
                "expected ParseError for fixture: {xml}, got: {res:?}"
            );
        }
    }

    #[test]
    fn accepts_single_root_with_comments_and_whitespace() {
        let valid_cases = [
            "<installer version=\"1.0.0\" />",
            "<!-- comment --><installer><modules /></installer><!-- after -->",
            "<?xml version=\"1.0\"?><installer />",
            "   \n\t <installer>\n</installer> \n\t ",
        ];

        for xml in valid_cases {
            let res = InstallManifest::parse(xml);
            assert!(
                res.is_ok(),
                "expected Ok for valid XML: {xml}, got: {res:?}"
            );
        }
    }

    #[test]
    fn test_enforces_dom_depth_limit_issue_336() {
        // Depth 64 must succeed: root <installer> (1) + 63 nested <eX> tags = 64
        let mut open_tags = String::from("<installer>");
        let mut close_tags = String::from("</installer>");
        for i in 1..=63 {
            open_tags.push_str(&format!("<e{i}>"));
            close_tags = format!("</e{i}>{close_tags}");
        }
        let valid_depth_xml = format!("{open_tags}{close_tags}");
        assert!(parse_xml_to_dom(&valid_depth_xml).is_ok());

        // Depth 65 must fail: root <installer> (1) + 64 nested <eX> tags = 65
        let mut open_tags_65 = String::from("<installer>");
        let mut close_tags_65 = String::from("</installer>");
        for i in 1..=64 {
            open_tags_65.push_str(&format!("<e{i}>"));
            close_tags_65 = format!("</e{i}>{close_tags_65}");
        }
        let invalid_depth_xml = format!("{open_tags_65}{close_tags_65}");
        let err = parse_xml_to_dom(&invalid_depth_xml).unwrap_err();
        assert!(matches!(err, InstallXmlError::ParseError { .. }));
        assert!(err.to_string().contains("nesting depth exceeded"));
    }

    #[test]
    fn test_enforces_element_count_limit_issue_336() {
        // 4096 elements: <installer> (1) + 4095 flat <m/> tags = 4096
        let mut valid_xml = String::with_capacity(32_768);
        valid_xml.push_str("<installer>");
        for _ in 1..=4095 {
            valid_xml.push_str("<m/>");
        }
        valid_xml.push_str("</installer>");
        assert!(parse_xml_to_dom(&valid_xml).is_ok());

        // 4097 elements: <installer> (1) + 4096 flat <m/> tags = 4097
        let mut invalid_xml = String::with_capacity(32_768);
        invalid_xml.push_str("<installer>");
        for _ in 1..=4096 {
            invalid_xml.push_str("<m/>");
        }
        invalid_xml.push_str("</installer>");
        let err = parse_xml_to_dom(&invalid_xml).unwrap_err();
        assert!(matches!(err, InstallXmlError::ParseError { .. }));
        assert!(err.to_string().contains("element count exceeded"));
    }

    #[test]
    fn test_enforces_source_size_and_text_limits_issue_336() {
        // Oversized source (> 512 KiB)
        let huge_xml = format!(
            "<installer>{}</installer>",
            " ".repeat(MAX_INSTALL_XML_BYTES + 10)
        );
        let err = InstallManifest::parse(&huge_xml).unwrap_err();
        assert!(matches!(err, InstallXmlError::ParseError { .. }));
        assert!(err.to_string().contains("size exceeds maximum limit"));

        // Oversized text node (> 64 KiB)
        let huge_text_elem = format!(
            "<installer><bot>{}</bot></installer>",
            "a".repeat(MAX_XML_ELEMENT_TEXT_BYTES + 10)
        );
        let err_text = parse_xml_to_dom(&huge_text_elem).unwrap_err();
        assert!(matches!(err_text, InstallXmlError::ParseError { .. }));
        assert!(
            err_text
                .to_string()
                .contains("text content exceeds maximum limit")
        );

        // Oversized CDATA (> 64 KiB)
        let huge_cdata = format!(
            "<installer><bot><![CDATA[{}]]></bot></installer>",
            "a".repeat(MAX_XML_ELEMENT_TEXT_BYTES + 10)
        );
        let err_cdata = parse_xml_to_dom(&huge_cdata).unwrap_err();
        assert!(matches!(err_cdata, InstallXmlError::ParseError { .. }));
        assert!(
            err_cdata
                .to_string()
                .contains("text content exceeds maximum limit")
        );
    }

    #[test]
    fn test_reject_duplicate_xml_attributes_issue_381() {
        // 1. Duplicate enabled attribute on module tag (false then true)
        let xml1 = r#"<installer><modules>
            <module id="demo" enabled="false" enabled="true">
                <folder>demo</folder><entry>module.lua</entry>
            </module>
        </modules></installer>"#;
        let err1 = InstallManifest::parse(xml1).unwrap_err();
        assert!(matches!(err1, InstallXmlError::ParseError { .. }));
        assert!(
            err1.to_string()
                .contains("duplicate XML attribute 'enabled'")
        );

        // 2. Duplicate enabled attribute in reverse order (true then false)
        let xml2 = r#"<installer><modules>
            <module id="demo" enabled="true" enabled="false">
                <folder>demo</folder><entry>module.lua</entry>
            </module>
        </modules></installer>"#;
        let err2 = InstallManifest::parse(xml2).unwrap_err();
        assert!(matches!(err2, InstallXmlError::ParseError { .. }));
        assert!(
            err2.to_string()
                .contains("duplicate XML attribute 'enabled'")
        );

        // 3. Duplicate id attribute
        let xml3 = r#"<installer><modules>
            <module id="first" id="second" enabled="true">
                <folder>demo</folder><entry>module.lua</entry>
            </module>
        </modules></installer>"#;
        let err3 = InstallManifest::parse(xml3).unwrap_err();
        assert!(matches!(err3, InstallXmlError::ParseError { .. }));
        assert!(err3.to_string().contains("duplicate XML attribute 'id'"));

        // 4. Duplicate root attribute
        let xml_root =
            r#"<installer version="1.0.0" version="2.0.0"><modules></modules></installer>"#;
        let err_root = InstallManifest::parse(xml_root).unwrap_err();
        assert!(matches!(err_root, InstallXmlError::ParseError { .. }));
        assert!(
            err_root
                .to_string()
                .contains("duplicate XML attribute 'version'")
        );

        // 5. Self-closing tag with duplicate attribute
        let xml_self_closing = r#"<installer><module id="demo" id="demo_dup"/></installer>"#;
        let err_sc = parse_xml_to_dom(xml_self_closing).unwrap_err();
        assert!(matches!(err_sc, InstallXmlError::ParseError { .. }));
        assert!(err_sc.to_string().contains("duplicate XML attribute 'id'"));
    }
}
