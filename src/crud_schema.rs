use std::fs;

/// Minimal declarative CRUD schema reader. This is intentionally independent
/// from the executable RUX parser until data declarations become first-class AST nodes.
pub fn generate(path: &str) -> Result<(), String> {
    if !path.ends_with(".rux") { return Err("expected a .rux file".into()); }
    let source = fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let (name, fields) = parse_typed_schema(&source)?;
    super::crud::create_typed(&name, &fields)
}

pub fn parse_schema(source: &str) -> Result<(String, Vec<String>), String> {
    let (name, fields) = parse_typed_schema(source)?;
    Ok((name, fields.into_iter().map(|(name, _)| name).collect()))
}

/// Converts supported date/time pattern tokens into chrono directives.
pub fn chrono_datetime_pattern(pattern: &str) -> Result<String, String> {
    if pattern.is_empty() || pattern.len() > 100 { return Err("format must contain 1 to 100 characters".into()); }
    let mut result = String::new();
    let mut has_component = false;
    let mut has_date = false;
    let mut has_time = false;
    let mut chars = pattern.chars().peekable();
    while let Some(ch) = chars.next() {
        if matches!(ch, 'y' | 'M' | 'm' | 'd' | 'H' | 'h' | 's' | 'S' | 'a') {
            let mut count = 1;
            while chars.peek() == Some(&ch) { chars.next(); count += 1; }
            let directive = match (ch, count) {
                ('y', 2) => "%y", ('y', 4) => "%Y",
                ('M', 1 | 2) => "%m", ('M', 3) => "%b", ('M', 4) => "%B",
                ('d', 1 | 2) => "%d",
                ('H', 1 | 2) => "%H", ('h', 1 | 2) => "%I",
                ('m', 1 | 2) => if pattern.contains('H') || pattern.contains('h') || pattern.contains('s') { "%M" } else { "%m" },
                ('s', 1 | 2) => "%S",
                ('S', 1..=9) => "%f",
                ('a', 1) => "%p",
                _ => return Err(format!("unsupported date/time token: {}", ch.to_string().repeat(count))),
            };
            has_component = true;
            has_date |= matches!(ch, 'y' | 'M' | 'd') || (ch == 'm' && directive == "%m");
            has_time |= matches!(ch, 'H' | 'h' | 's' | 'S' | 'a') || (ch == 'm' && directive == "%M");
            result.push_str(directive);
        } else if matches!(ch, '-' | '/' | '.' | ':' | ' ' | 'T' | ',' | '_') {
            result.push(ch);
        } else {
            return Err(format!("unsupported date/time format character: {ch}"));
        }
    }
    if !has_component || (!has_date && !has_time) { return Err("format must contain date or time tokens".into()); }
    Ok(result)
}

pub fn parse_typed_schema(source: &str) -> Result<(String, Vec<(String, String)>), String> {
    let mut app = None;
    let mut entity = None;
    let mut fields = Vec::new();
    let mut depth = 0usize;
    let mut entity_depth = None;
    let mut app_depth = None;
    for (index, raw) in source.lines().enumerate() {
        let line = raw.split("//").next().unwrap_or("").trim();
        if line.is_empty() { continue; }
        let open = line.chars().filter(|&c| c == '{').count();
        let close = line.chars().filter(|&c| c == '}').count();
        if open > 1 || close > 1 || (open > 0 && close > 0) {
            return Err(format!("line {}: one declaration or brace per line", index + 1));
        }
        if app.is_none() {
            let parts: Vec<_> = line.split_whitespace().collect();
            if parts.len() != 3 || parts[0] != "app" || parts[2] != "{" {
                return Err(format!("line {}: expected app Name {{", index + 1));
            }
            app = Some(parts[1].to_string());
            app_depth = Some(1);
            depth = 1;
            continue;
        }
        if line == "}" {
            if depth == 0 { return Err(format!("line {}: unexpected closing brace", index + 1)); }
            if entity_depth == Some(depth) { entity_depth = None; }
            depth -= 1;
            continue;
        }
        if depth == 0 { return Err(format!("line {}: content after app", index + 1)); }
        if line.starts_with("data ") {
            if entity.is_some() || depth != 1 || !line.ends_with('{') {
                return Err(format!("line {}: expected one data entity", index + 1));
            }
            let parts: Vec<_> = line.split_whitespace().collect();
            if parts.len() != 3 || parts[2] != "{" {
                return Err(format!("line {}: expected data Name {{", index + 1));
            }
            entity = Some(parts[1].to_string());
            depth += 1;
            entity_depth = Some(depth);
            continue;
        }
        if entity_depth == Some(depth) && open == 0 && close == 0 {
            let (field, ty) = line.split_once(':').ok_or_else(|| format!("line {}: expected field: Type", index + 1))?;
            let field = field.trim();
            let ty = ty.trim();
            let (ty, regex_pattern) = if let Some((base, expression)) = ty.split_once(", regex(") {
                let expression = expression.strip_suffix(')')
                    .ok_or_else(|| format!("line {}: invalid regex declaration", index + 1))?;
                let pattern = expression.strip_prefix('"').and_then(|s| s.strip_suffix('"'))
                    .ok_or_else(|| format!("line {}: regex must be quoted", index + 1))?;
                if pattern.is_empty() || pattern.len() > 200
                    || !pattern.starts_with('^') || !pattern.ends_with('$')
                    || pattern.contains('"') || pattern.contains(char::from(92))
                {
                    return Err(format!("line {}: regex must be anchored, at most 200 characters, without backslashes or quotes", index + 1));
                }
                regex::Regex::new(pattern)
                    .map_err(|error| format!("line {}: invalid regex: {error}", index + 1))?;
                (base.trim(), Some(pattern))
            } else {
                (ty, None)
            };
            let ty = if let Some(pattern) = ty.strip_prefix("Text,") {
                let pattern = pattern.trim();
                if !pattern.starts_with('"') || !pattern.ends_with('"') || pattern.len() < 3 {
                    return Err(format!("line {}: Text mask requires a quoted pattern", index + 1));
                }
                let pattern = &pattern[1..pattern.len() - 1];
                if pattern.len() > 64 || !pattern.contains('9') || !pattern.chars().all(|ch| ch == '9' || " .-/()+".contains(ch)) {
                    return Err(format!("line {}: invalid Text mask (use 9 for digits)", index + 1));
                }
                format!("Text|{pattern}")
            } else if let Some(pattern) = ty.strip_prefix("DateTime,") {
                let pattern = pattern.trim();
                if !pattern.starts_with('"') || !pattern.ends_with('"') || pattern.len() < 3 {
                    return Err(format!("line {}: DateTime requires a quoted format", index + 1));
                }
                let pattern = &pattern[1..pattern.len() - 1];
                chrono_datetime_pattern(pattern)
                    .map_err(|error| format!("line {}: invalid DateTime format: {error}", index + 1))?;
                format!("DateTime|{pattern}")
            } else { ty.to_string() };
            if !matches!(ty.as_str(), "Text" | "Email" | "Int" | "Decimal" | "Bool" | "Date") && !ty.starts_with("DateTime|") && !ty.starts_with("Text|") {
                return Err(format!("line {}: unsupported field type: {ty}", index + 1));
            }
            if regex_pattern.is_some() && ty != "Text" && !ty.starts_with("Text|") {
                return Err(format!("line {}: regex is currently supported only for Text", index + 1));
            }
            let ty = match regex_pattern { Some(pattern) => format!("{ty}~{pattern}"), None => ty };
            fields.push((field.to_string(), ty));
            continue;
        }
        return Err(format!("line {}: unsupported declaration", index + 1));
    }
    if depth != 0 || app_depth.is_none() || entity.is_none() || fields.is_empty() {
        return Err("expected a closed app containing one data entity with fields".into());
    }
    let name = app.unwrap();
    if !name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("invalid app name".into());
    }
    let mut unique = std::collections::HashSet::new();
    if fields.len() > 20 || fields.iter().any(|(f, _)| f == "id" || !f.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) || !f.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || !unique.insert(f.clone())) {
        return Err("invalid, duplicate or reserved field (maximum 20)".into());
    }
    Ok((name, fields))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_schema() {
        let src = "app Clientes {\n data Cliente {\n nome: Text\n email: Email\n idade: Int\n }\n}";
        assert_eq!(parse_schema(src).unwrap(), ("Clientes".into(), vec!["nome".to_string(), "email".to_string(), "idade".to_string()]));
    }
    #[test]
    fn rejects_invalid_type() {
        assert!(parse_schema("app A {\n data X {\n a: Unknown\n }\n}").is_err());
    }
    #[test]
    fn rejects_unclosed_app() {
        assert!(parse_schema("app A {\n data X {\n a: Text\n }").is_err());
    }
}
