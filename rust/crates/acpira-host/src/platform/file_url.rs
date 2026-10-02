//! Local file URLs used by attachments, tool output, file search and IDE navigation.
//! The platform-explicit core keeps drive / UNC cases testable on non-Windows hosts.

pub fn is_file_url(value: &str) -> bool {
  value.get(..5).is_some_and(|prefix| prefix.eq_ignore_ascii_case("file:"))
}

pub fn path_to_file_url(path: &str) -> String {
  encode(path, cfg!(windows))
}

pub fn file_url_to_path(uri: &str) -> Option<String> {
  decode(uri, cfg!(windows))
}

fn encode(path: &str, windows: bool) -> String {
  let path = if windows {
    let path = path.replace('\\', "/");
    // canonicalize() returns extended-length paths on Windows; the prefix is not part of a file URL.
    if path.get(..8).is_some_and(|p| p.eq_ignore_ascii_case("//?/UNC/")) {
      format!("//{}", &path[8..])
    } else {
      path.strip_prefix("//?/").unwrap_or(&path).to_owned()
    }
  } else {
    path.to_owned()
  };
  let (prefix, path) = if windows {
    if let Some(unc) = path.strip_prefix("//") {
      let (host, rest) = unc.split_once('/').unwrap_or((unc, ""));
      (format!("file://{}/", host.to_ascii_lowercase()), rest)
    } else {
      ("file:///".to_owned(), path.as_str())
    }
  } else {
    ("file://".to_owned(), path.as_str())
  };
  let mut out = prefix;
  for b in path.bytes() {
    if b.is_ascii_alphanumeric() || b"/-._~!$&'()*+,;=:@".contains(&b) {
      out.push(b as char);
    } else {
      out.push_str(&format!("%{b:02X}"));
    }
  }
  out
}

fn decode(uri: &str, windows: bool) -> Option<String> {
  if !is_file_url(uri) {
    return None;
  }
  let rest = uri.get(5..)?.split(['?', '#']).next()?;
  let (host, path) = if let Some(rest) = rest.strip_prefix("//") {
    let i = rest.find('/')?;
    (&rest[..i], &rest[i..])
  } else {
    ("", rest)
  };
  if !path.starts_with('/') || host.contains(['@', ':', '\\', '%']) {
    return None;
  }
  let decoded = percent_decode(path, windows)?;
  if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
    return windows.then(|| format!("\\\\{}{}", host.to_ascii_lowercase(), decoded.replace('/', "\\")));
  }
  if !windows {
    return Some(decoded);
  }
  let path = decoded.strip_prefix('/')?;
  let bytes = path.as_bytes();
  // A local Windows file URL needs an absolute drive path. UNC paths require an authority.
  if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' || bytes[2] != b'/' {
    return None;
  }
  Some(path.replace('/', "\\"))
}

/// Decode local path text without allowing an encoded separator to change its directory structure.
pub fn percent_decode(s: &str, windows: bool) -> Option<String> {
  let bytes = s.as_bytes();
  let mut out = Vec::with_capacity(bytes.len());
  let mut i = 0;
  while i < bytes.len() {
    let value = if bytes[i] == b'%' {
      let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
      let value = u8::from_str_radix(hex, 16).ok()?;
      if value == b'/' || (windows && value == b'\\') {
        return None;
      }
      i += 3;
      value
    } else {
      let value = bytes[i];
      i += 1;
      value
    };
    if value == 0 {
      return None;
    }
    out.push(value);
  }
  String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn windows_drive_unc_and_canonical_paths_match_nodes_file_url_conversion() {
    for (path, url, decoded) in [
      (r"C:\work\中文 #?%.png", "file:///C:/work/%E4%B8%AD%E6%96%87%20%23%3F%25.png", r"C:\work\中文 #?%.png"),
      (r"\\server\share\a b.txt", "file://server/share/a%20b.txt", r"\\server\share\a b.txt"),
      (r"\\?\C:\work\a.txt", "file:///C:/work/a.txt", r"C:\work\a.txt"),
      (r"\\?\UNC\Server\share\a.txt", "file://server/share/a.txt", r"\\server\share\a.txt"),
      ("C:/a.txt", "file:///C:/a.txt", r"C:\a.txt"),
    ] {
      assert_eq!(encode(path, true), url, "{path}");
      assert_eq!(decode(url, true).as_deref(), Some(decoded), "{url}");
    }
  }

  #[test]
  fn decoding_checks_scheme_authority_and_separators() {
    assert_eq!(decode("FILE://LOCALHOST/C:/a%23b.txt?q#L2", true).as_deref(), Some(r"C:\a#b.txt"));
    assert_eq!(decode("file:/C:/a.txt", true).as_deref(), Some(r"C:\a.txt"));
    for uri in [
      "file:///C:/a%5Cb.txt",
      "file:///C:/a%2fb.txt",
      "file:///C:/a%00b.txt",
      "https:///C:/a.txt",
      "file:///no-drive",
      "file://user@host/a",
    ] {
      assert_eq!(decode(uri, true), None, "{uri}");
    }
    assert!(!is_file_url("中文图片.png"));
    assert_eq!(decode("file://server/share/a", false), None);
  }

  #[test]
  fn posix_paths_preserve_literal_backslashes_and_encoded_filename_punctuation() {
    let path = "/tmp/中文 #?%\\.png";
    let url = "file:///tmp/%E4%B8%AD%E6%96%87%20%23%3F%25%5C.png";
    assert_eq!(encode(path, false), url);
    assert_eq!(decode(url, false).as_deref(), Some(path));
    assert_eq!(decode("file://LOCALHOST/tmp/a?q#L3", false).as_deref(), Some("/tmp/a"));
  }
}
