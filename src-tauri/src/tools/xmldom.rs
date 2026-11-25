//! Tiny XML DOM for Office files (docx_engine.rs).
//!
//! Why our own instead of a crate: we must write the XML back with every element,
//! attribute, namespace prefix and unknown extension (w14:, mc:, ...) exactly as Word
//! wrote it. This keeps names and attributes verbatim and never expands DTD entities
//! (so no XXE / billion-laughs). Office XML has no DTDs anyway.

#[derive(Clone, Debug)]
pub enum Node {
    El(El),
    /// Character data, stored UNescaped.
    Text(String),
    /// Comments, processing instructions, declarations: written back verbatim.
    Raw(String),
}

#[derive(Clone, Debug, Default)]
pub struct El {
    pub name: String,
    pub attrs: Vec<(String, String)>, // values stored unescaped
    pub kids: Vec<Node>,
}

pub struct Doc {
    pub prolog: Vec<Node>,
    pub root: El,
}

const MAX_DEPTH: usize = 400;

impl El {
    pub fn new(name: &str) -> El {
        El { name: name.to_string(), attrs: Vec::new(), kids: Vec::new() }
    }
    /// Builder: add an attribute.
    pub fn a(mut self, k: &str, v: &str) -> El {
        self.set_attr(k, v);
        self
    }
    /// Builder: add a child element.
    pub fn c(mut self, child: El) -> El {
        self.kids.push(Node::El(child));
        self
    }
    /// Builder: add a text child.
    pub fn t(mut self, text: &str) -> El {
        self.kids.push(Node::Text(text.to_string()));
        self
    }
    pub fn attr(&self, k: &str) -> Option<&str> {
        self.attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
    }
    pub fn set_attr(&mut self, k: &str, v: &str) {
        if let Some(slot) = self.attrs.iter_mut().find(|(n, _)| n == k) {
            slot.1 = v.to_string();
        } else {
            self.attrs.push((k.to_string(), v.to_string()));
        }
    }
    pub fn elems(&self) -> impl Iterator<Item = &El> {
        self.kids.iter().filter_map(|n| if let Node::El(e) = n { Some(e) } else { None })
    }
    pub fn elems_mut(&mut self) -> impl Iterator<Item = &mut El> {
        self.kids.iter_mut().filter_map(|n| if let Node::El(e) = n { Some(e) } else { None })
    }
    pub fn child(&self, name: &str) -> Option<&El> {
        self.elems().find(|e| e.name == name)
    }
    pub fn child_mut(&mut self, name: &str) -> Option<&mut El> {
        self.elems_mut().find(|e| e.name == name)
    }
    pub fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a El> + 'a {
        self.elems().filter(move |e| e.name == name)
    }
    pub fn has(&self, name: &str) -> bool {
        self.child(name).is_some()
    }
    pub fn remove_children(&mut self, name: &str) {
        self.kids.retain(|n| !matches!(n, Node::El(e) if e.name == name));
    }
    /// All descendant elements with this name (depth-first, document order).
    pub fn find_all<'a>(&'a self, name: &str, out: &mut Vec<&'a El>) {
        for e in self.elems() {
            if e.name == name {
                out.push(e);
            }
            e.find_all(name, out);
        }
    }
    /// Any descendant with this name?
    pub fn contains(&self, name: &str) -> bool {
        self.elems().any(|e| e.name == name || e.contains(name))
    }
    /// Element at a path of indexes into `kids`.
    pub fn at_path(&self, path: &[usize]) -> Option<&El> {
        let mut cur = self;
        for &i in path {
            cur = match cur.kids.get(i) {
                Some(Node::El(e)) => e,
                _ => return None,
            };
        }
        Some(cur)
    }
    pub fn at_path_mut(&mut self, path: &[usize]) -> Option<&mut El> {
        let mut cur = self;
        for &i in path {
            cur = match cur.kids.get_mut(i) {
                Some(Node::El(e)) => e,
                _ => return None,
            };
        }
        Some(cur)
    }
}

// ------------------------------------------------------------------ parse

pub fn parse(src: &str) -> Result<Doc, String> {
    let s = src.strip_prefix('\u{feff}').unwrap_or(src);
    let b = s.as_bytes();
    let mut i = 0usize;
    let mut stack: Vec<El> = Vec::new();
    let mut prolog: Vec<Node> = Vec::new();
    let mut root: Option<El> = None;

    fn push_node(stack: &mut Vec<El>, prolog: &mut Vec<Node>, n: Node) {
        match stack.last_mut() {
            Some(top) => top.kids.push(n),
            None => {
                // whitespace / comments outside the root: keep only non-text
                if !matches!(n, Node::Text(_)) {
                    prolog.push(n);
                }
            }
        }
    }
    let find = |from: usize, pat: &str| -> Result<usize, String> {
        s[from..].find(pat).map(|p| from + p).ok_or_else(|| format!("XML: missing '{}'", pat))
    };

    while i < b.len() {
        if b[i] == b'<' {
            if s[i..].starts_with("<?") {
                let end = find(i, "?>")? + 2;
                push_node(&mut stack, &mut prolog, Node::Raw(s[i..end].to_string()));
                i = end;
            } else if s[i..].starts_with("<!--") {
                let end = find(i, "-->")? + 3;
                push_node(&mut stack, &mut prolog, Node::Raw(s[i..end].to_string()));
                i = end;
            } else if s[i..].starts_with("<![CDATA[") {
                let end = find(i, "]]>")?;
                push_node(&mut stack, &mut prolog, Node::Text(s[i + 9..end].to_string()));
                i = end + 3;
            } else if s[i..].starts_with("<!") {
                // DOCTYPE: kept verbatim, entities are never expanded
                let end = find(i, ">")? + 1;
                push_node(&mut stack, &mut prolog, Node::Raw(s[i..end].to_string()));
                i = end;
            } else if s[i..].starts_with("</") {
                let end = find(i, ">")?;
                let name = s[i + 2..end].trim();
                let el = stack.pop().ok_or("XML: unexpected closing tag")?;
                if el.name != name {
                    return Err(format!("XML: </{}> does not match <{}>", name, el.name));
                }
                i = end + 1;
                match stack.last_mut() {
                    Some(top) => top.kids.push(Node::El(el)),
                    None => {
                        if root.is_some() {
                            return Err("XML: more than one root element".into());
                        }
                        root = Some(el)
                    }
                }
            } else {
                // start or empty tag
                let mut j = i + 1;
                while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' && b[j] != b'/' {
                    j += 1;
                }
                let mut el = El::new(&s[i + 1..j]);
                if el.name.is_empty() {
                    return Err("XML: empty tag name".into());
                }
                loop {
                    while j < b.len() && b[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if j >= b.len() {
                        return Err("XML: unterminated tag".into());
                    }
                    if b[j] == b'>' || (b[j] == b'/' && j + 1 < b.len() && b[j + 1] == b'>') {
                        break;
                    }
                    let k0 = j;
                    while j < b.len() && b[j] != b'=' && !b[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    let key = s[k0..j].to_string();
                    while j < b.len() && b[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if j >= b.len() || b[j] != b'=' {
                        return Err(format!("XML: attribute '{}' has no value", key));
                    }
                    j += 1;
                    while j < b.len() && b[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if j >= b.len() || (b[j] != b'"' && b[j] != b'\'') {
                        return Err("XML: attribute value must be quoted".into());
                    }
                    let q = b[j] as char;
                    let v0 = j + 1;
                    let v1 = s[v0..].find(q).map(|p| v0 + p).ok_or("XML: unterminated attribute")?;
                    el.attrs.push((key, unescape(&s[v0..v1])));
                    j = v1 + 1;
                }
                if b[j] == b'/' {
                    i = j + 2;
                    match stack.last_mut() {
                        Some(top) => top.kids.push(Node::El(el)),
                        None => root = Some(el),
                    }
                } else {
                    i = j + 1;
                    if stack.len() >= MAX_DEPTH {
                        return Err("XML: nested too deeply".into());
                    }
                    stack.push(el);
                }
            }
        } else {
            let end = s[i..].find('<').map(|p| i + p).unwrap_or(b.len());
            push_node(&mut stack, &mut prolog, Node::Text(unescape(&s[i..end])));
            i = end;
        }
    }
    if !stack.is_empty() {
        return Err(format!("XML: <{}> is never closed", stack.last().unwrap().name));
    }
    let root = root.ok_or("XML: no root element")?;
    Ok(Doc { prolog, root })
}

pub fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let semi = match rest.find(';') {
            Some(x) if x <= 12 => x,
            _ => {
                out.push('&');
                rest = &rest[1..];
                continue;
            }
        };
        let ent = &rest[1..semi];
        let rep = match ent {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32)
            }
            _ if ent.starts_with('#') => ent[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match rep {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

// ------------------------------------------------------------------ write

fn esc_text(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#13;"),
            // control characters are not allowed in XML 1.0
            c if (c as u32) < 0x20 && c != '\n' && c != '\t' => {}
            c => out.push(c),
        }
    }
}

fn esc_attr(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' => out.push_str("&#9;"),
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
}

pub fn write_el(e: &El, out: &mut String) {
    out.push('<');
    out.push_str(&e.name);
    for (k, v) in &e.attrs {
        out.push(' ');
        out.push_str(k);
        out.push_str("=\"");
        esc_attr(v, out);
        out.push('"');
    }
    if e.kids.is_empty() {
        out.push_str("/>");
        return;
    }
    out.push('>');
    for n in &e.kids {
        match n {
            Node::El(c) => write_el(c, out),
            Node::Text(t) => esc_text(t, out),
            Node::Raw(r) => out.push_str(r),
        }
    }
    out.push_str("</");
    out.push_str(&e.name);
    out.push('>');
}

pub fn to_string(d: &Doc) -> String {
    let mut out = String::new();
    let mut has_decl = false;
    for n in &d.prolog {
        if let Node::Raw(r) = n {
            if r.starts_with("<?xml ") {
                has_decl = true;
            }
            out.push_str(r);
            if r.starts_with("<?xml ") {
                out.push_str("\r\n");
            }
        }
    }
    if !has_decl {
        out.insert_str(0, "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n");
    }
    write_el(&d.root, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_round_trip_keeps_everything() {
        let src = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n<w:document xmlns:w=\"x\" mc:Ignorable=\"w14\"><!-- c --><w:body><w:p w14:paraId=\"1A\"><w:r><w:t xml:space=\"preserve\"> a &amp; b &lt;c&gt; </w:t></w:r><w:r/></w:p></w:body></w:document>";
        let d = parse(src).unwrap();
        assert_eq!(to_string(&d), src);
        let t = &d.root.at_path(&[1, 0, 0, 0]).unwrap().kids[0];
        assert!(matches!(t, Node::Text(s) if s == " a & b <c> "));
    }

    #[test]
    fn xml_rejects_broken_and_never_expands_entities() {
        assert!(parse("<a><b></a>").is_err());
        assert!(parse("<a>").is_err());
        let d = parse("<!DOCTYPE a [<!ENTITY x \"boom\">]><a>&x;</a>").unwrap();
        assert!(matches!(&d.root.kids[0], Node::Text(s) if s == "&x;"));
    }
}