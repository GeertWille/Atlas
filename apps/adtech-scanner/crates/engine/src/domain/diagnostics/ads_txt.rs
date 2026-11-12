//! Parser for ads.txt files, following the IAB Tech Lab ads.txt 1.1
//! specification: comments, seller records, variables and extension fields.

use super::excerpt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relationship {
    Direct,
    Reseller,
}

impl Relationship {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "DIRECT",
            Self::Reseller => "RESELLER",
        }
    }
}

/// One authorised seller: `<ad system domain>, <account id>, <relationship>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// 1-based line number.
    pub line: u32,
    /// Lower-cased ad system domain.
    pub domain: String,
    pub account_id: String,
    pub relationship: Relationship,
    pub cert_authority_id: Option<String>,
    /// The record as written, without its comment.
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidLine {
    pub line: u32,
    pub text: String,
    pub problem: &'static str,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdsTxt {
    pub records: Vec<Record>,
    pub invalid: Vec<InvalidLine>,
    pub owner_domain: Option<String>,
    pub manager_domains: Vec<String>,
    /// Subdomains whose own ads.txt replaces this one for their pages.
    pub subdomains: Vec<String>,
}

impl AdsTxt {
    /// The records of one ad system, given the domains it is listed under.
    pub fn records_for<'a>(&'a self, domains: &'a [&str]) -> impl Iterator<Item = &'a Record> {
        self.records
            .iter()
            .filter(|record| domains.contains(&record.domain.as_str()))
    }
}

/// What a site's answer to the ads.txt request means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdsTxtOutcome {
    Found(AdsTxt),
    /// The site says it has no such file.
    Missing,
    /// No conclusion is possible; the text says why.
    Unreadable(String),
}

/// Reads an HTTP answer as ads.txt. Only "not found" and "gone" mean the site
/// has no file: sites answer crawlers they do not know with 403, challenge
/// pages and error pages, none of which say anything about ads.txt.
pub fn read_response(status: u16, content_type: Option<&str>, body: &[u8]) -> AdsTxtOutcome {
    match status {
        404 | 410 => return AdsTxtOutcome::Missing,
        200..=299 => {}
        other => return AdsTxtOutcome::Unreadable(format!("the site answered HTTP {other}")),
    }
    // Compressed or otherwise binary data would parse as nothing but invalid lines.
    if body.contains(&0) || body.starts_with(&[0x1f, 0x8b]) {
        return AdsTxtOutcome::Unreadable("the site answered with data that is not text".into());
    }
    let text = String::from_utf8_lossy(body);
    let media_type = content_type
        .and_then(|value| value.split(';').next())
        .map(|value| value.trim().to_ascii_lowercase());
    // The specification asks for text/plain. Real files are also served as
    // octet-stream or without a type, so only a body that is plainly a web
    // page is turned away.
    if media_type.as_deref() == Some("text/html") || looks_like_markup(&text) {
        return AdsTxtOutcome::Unreadable(
            "the site answered with a web page instead of a text file".into(),
        );
    }
    let parsed = parse(&text);
    // Buyers treat a file without a single record like a missing one: an
    // error page, a soft 404 or an empty file authorises and forbids nobody.
    // A site that means "nobody is authorised" publishes the placeholder
    // record of the specification, which is a record.
    if parsed.records.is_empty() {
        return AdsTxtOutcome::Unreadable(
            if parsed.invalid.is_empty() {
                "the file has no seller records"
            } else {
                "none of the lines in the answer is an ads.txt record"
            }
            .into(),
        );
    }
    AdsTxtOutcome::Found(parsed)
}

/// No ads.txt line starts with a tag.
fn looks_like_markup(text: &str) -> bool {
    text.trim_start_matches('\u{feff}')
        .trim_start()
        .starts_with('<')
}

pub fn parse(text: &str) -> AdsTxt {
    let mut parsed = AdsTxt::default();
    // Lines end in LF, CR or CRLF.
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    for (index, raw) in text.split(['\n', '\r']).enumerate() {
        let line = index as u32 + 1;
        let content = raw.split('#').next().unwrap_or_default().trim();
        if content.is_empty() {
            continue;
        }
        if let Some((name, value)) = variable(content) {
            parsed.set_variable(&name, value);
            continue;
        }
        match record(line, content) {
            Ok(record) => {
                // A line repeated in the file authorises the seller once.
                let is_repeated = parsed.records.iter().any(|known| {
                    known.domain == record.domain
                        && known.account_id == record.account_id
                        && known.relationship == record.relationship
                });
                if !is_repeated {
                    parsed.records.push(record);
                }
            }
            Err(problem) => parsed.invalid.push(InvalidLine {
                line,
                text: excerpt(content),
                problem,
            }),
        }
    }
    parsed
}

/// `NAME=value`, where the name has no comma: a record's extension field may
/// contain `=` too, but only after its commas.
fn variable(content: &str) -> Option<(String, &str)> {
    let (name, value) = content.split_once('=')?;
    let name = name.trim();
    let is_name = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    is_name.then(|| (name.to_ascii_uppercase(), value.trim()))
}

impl AdsTxt {
    /// Variables this parser has no use for (CONTACT, INVENTORYPARTNERDOMAIN,
    /// future ones) are valid lines and are skipped.
    fn set_variable(&mut self, name: &str, value: &str) {
        // MANAGERDOMAIN may carry a country code: `domain, CC`.
        let Some(domain) = value
            .split(',')
            .next()
            .map(|domain| domain.trim().to_ascii_lowercase())
            .filter(|domain| is_domain(domain))
        else {
            return;
        };
        match name {
            "OWNERDOMAIN" => {
                self.owner_domain.get_or_insert(domain);
            }
            "MANAGERDOMAIN" => push_unique(&mut self.manager_domains, domain),
            "SUBDOMAIN" => push_unique(&mut self.subdomains, domain),
            _ => {}
        }
    }
}

fn push_unique(list: &mut Vec<String>, value: String) {
    if !list.contains(&value) {
        list.push(value);
    }
}

fn record(line: u32, content: &str) -> Result<Record, &'static str> {
    // Extension fields follow the first semicolon and are not interpreted.
    let data = content.split(';').next().unwrap_or_default();
    let mut fields: Vec<&str> = data.split(',').map(str::trim).collect();
    // Trailing commas are common in real files and change nothing.
    while fields.len() > 4 && fields.last().is_some_and(|field| field.is_empty()) {
        fields.pop();
    }
    if fields.len() < 3 {
        return Err("a record needs an ad system domain, an account id and DIRECT or RESELLER");
    }
    if fields.len() > 4 {
        return Err("a record has at most four comma-separated fields");
    }
    let domain = fields[0].to_ascii_lowercase();
    if !is_domain(&domain) {
        return Err("the first field is not a domain name");
    }
    if fields[1].is_empty() {
        return Err("the account id is empty");
    }
    let relationship = match fields[2].to_ascii_uppercase().as_str() {
        "DIRECT" => Relationship::Direct,
        "RESELLER" => Relationship::Reseller,
        _ => return Err("the third field must be DIRECT or RESELLER"),
    };
    Ok(Record {
        line,
        domain,
        account_id: fields[1].to_string(),
        relationship,
        cert_authority_id: fields
            .get(3)
            .filter(|id| !id.is_empty())
            .map(|id| id.to_string()),
        text: excerpt(data.trim()),
    })
}

fn is_domain(text: &str) -> bool {
    text.contains('.')
        && !text.starts_with('.')
        && !text.ends_with('.')
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\u{feff}# ads.txt of example.com\r\n\
OWNERDOMAIN=Example-Media.com\r\n\
managerdomain=manager.example, DE\r\n\
MANAGERDOMAIN=other-manager.example\r\n\
CONTACT=adops@example.com\r\n\
subdomain=games.example.com\r\n\
\r\n\
AppNexus.com, 958, direct, f5ab79cb980f11d1  # header bidding\r\n\
rubiconproject.com,14062,RESELLER\r\n\
pubmatic.com , 160262 , DIRECT ; extension=1, more\r\n\
google.com\tpub-123\tDIRECT\r\n\
openx.com, 540191398, PARTNER\r\n\
not a domain, 12, DIRECT\r\n\
criteo.com, , DIRECT\r\n\
a.com, 1, DIRECT, cert, extra\r\n\
placeholder.example.com, placeholder, DIRECT, placeholder\r\n";

    #[test]
    fn records_variables_comments_and_extensions_are_parsed() {
        let parsed = parse(SAMPLE);
        let records: Vec<(u32, &str, &str, Relationship, Option<&str>)> = parsed
            .records
            .iter()
            .map(|record| {
                (
                    record.line,
                    record.domain.as_str(),
                    record.account_id.as_str(),
                    record.relationship,
                    record.cert_authority_id.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            records,
            [
                (
                    8,
                    "appnexus.com",
                    "958",
                    Relationship::Direct,
                    Some("f5ab79cb980f11d1")
                ),
                (
                    9,
                    "rubiconproject.com",
                    "14062",
                    Relationship::Reseller,
                    None
                ),
                (10, "pubmatic.com", "160262", Relationship::Direct, None),
                (
                    16,
                    "placeholder.example.com",
                    "placeholder",
                    Relationship::Direct,
                    Some("placeholder")
                ),
            ]
        );
        assert_eq!(
            parsed.records[0].text,
            "AppNexus.com, 958, direct, f5ab79cb980f11d1"
        );
        assert_eq!(parsed.owner_domain.as_deref(), Some("example-media.com"));
        assert_eq!(
            parsed.manager_domains,
            ["manager.example", "other-manager.example"]
        );
        assert_eq!(parsed.subdomains, ["games.example.com"]);
    }

    #[test]
    fn malformed_lines_are_reported_with_their_line_and_reason() {
        let parsed = parse(SAMPLE);
        let invalid: Vec<(u32, &str, &str)> = parsed
            .invalid
            .iter()
            .map(|line| (line.line, line.text.as_str(), line.problem))
            .collect();
        assert_eq!(
            invalid,
            [
                (
                    11,
                    "google.com\tpub-123\tDIRECT",
                    "a record needs an ad system domain, an account id and DIRECT or RESELLER"
                ),
                (
                    12,
                    "openx.com, 540191398, PARTNER",
                    "the third field must be DIRECT or RESELLER"
                ),
                (
                    13,
                    "not a domain, 12, DIRECT",
                    "the first field is not a domain name"
                ),
                (14, "criteo.com, , DIRECT", "the account id is empty"),
                (
                    15,
                    "a.com, 1, DIRECT, cert, extra",
                    "a record has at most four comma-separated fields"
                ),
            ]
        );
    }

    #[test]
    fn line_endings_of_every_platform_are_understood() {
        for text in [
            "a.com, 1, DIRECT\nb.com, 2, RESELLER",
            "a.com, 1, DIRECT\r\nb.com, 2, RESELLER\r\n",
            "a.com, 1, DIRECT\rb.com, 2, RESELLER",
        ] {
            let parsed = parse(text);
            assert_eq!(parsed.records.len(), 2, "{text:?}");
            assert!(parsed.invalid.is_empty(), "{text:?}");
        }
    }

    #[test]
    fn an_empty_or_comment_only_file_is_a_file_without_records() {
        for text in ["", "\n\n", "# nothing authorised\n   \n#\n"] {
            assert_eq!(parse(text), AdsTxt::default(), "{text:?}");
        }
    }

    #[test]
    fn the_shapes_real_files_come_in_are_read_as_the_spec_says() {
        /// Line, and the record it is: domain, account id, relationship.
        type Case = (
            &'static str,
            Option<(&'static str, &'static str, Relationship)>,
        );
        let direct = Relationship::Direct;
        let cases: [Case; 22] = [
            ("a.com,1,DIRECT", Some(("a.com", "1", direct))),
            ("  a.com ,\t1 , direct  ", Some(("a.com", "1", direct))),
            ("A.COM, AbC-1, Direct", Some(("a.com", "AbC-1", direct))),
            ("a.com, 1, DIRECT,", Some(("a.com", "1", direct))),
            (
                "a.com, 1, DIRECT, f08c47fec0942fa0,",
                Some(("a.com", "1", direct)),
            ),
            ("a.com, 1, DIRECT, cert, , ,", Some(("a.com", "1", direct))),
            (
                "a.com, 1, DIRECT # comment, with, commas",
                Some(("a.com", "1", direct)),
            ),
            (
                "a.com, 1, DIRECT; ext=a,b,c; other=1",
                Some(("a.com", "1", direct)),
            ),
            ("a.com, 1, DIRECT, cert ;ext", Some(("a.com", "1", direct))),
            ("a.com, pub-1 2, DIRECT", Some(("a.com", "pub-1 2", direct))),
            (
                "sub.a-b.co.uk, 1, RESELLER",
                Some(("sub.a-b.co.uk", "1", Relationship::Reseller)),
            ),
            ("a.com, 1, DIRECT, cert, extra", None),
            ("a.com, 1", None),
            ("a.com; 1; DIRECT", None),
            ("a.com 1 DIRECT", None),
            ("https://a.com, 1, DIRECT", None),
            ("a.com/path, 1, DIRECT", None),
            ("localhost, 1, DIRECT", None),
            (".a.com, 1, DIRECT", None),
            ("a.com., 1, DIRECT", None),
            ("a.com, 1, BOTH", None),
            ("<p>a.com, 1, DIRECT</p>", None),
        ];
        for (line, expected) in cases {
            let parsed = parse(line);
            let record = parsed.records.first().map(|record| {
                (
                    record.domain.as_str(),
                    record.account_id.as_str(),
                    record.relationship,
                )
            });
            assert_eq!(record, expected, "{line:?}");
            assert_eq!(
                parsed.invalid.len(),
                usize::from(expected.is_none()),
                "{line:?}"
            );
        }
    }

    #[test]
    fn variables_are_told_from_records_and_read_without_regard_to_case() {
        let parsed = parse(
            "OwnerDomain = Owner.Example \n\
             OWNERDOMAIN=second.example\n\
             MANAGERDOMAIN=m.example\nmanagerdomain=M.example, US\n\
             SUBDOMAIN=a.site.example\nSUBDOMAIN=not a domain\n\
             INVENTORYPARTNERDOMAIN=partner.example\nCONTACT=https://site.example/contact?a=b\n\
             FUTURE_VARIABLE=anything, at all\n\
             =nameless\n\
             a.com, 1, DIRECT; k=v\n",
        );
        assert_eq!(parsed.owner_domain.as_deref(), Some("owner.example"));
        assert_eq!(parsed.manager_domains, ["m.example"]);
        assert_eq!(parsed.subdomains, ["a.site.example"]);
        assert_eq!(parsed.records.len(), 1);
        let invalid: Vec<&str> = parsed
            .invalid
            .iter()
            .map(|line| line.text.as_str())
            .collect();
        assert_eq!(invalid, ["=nameless"]);
    }

    #[test]
    fn a_repeated_record_counts_once() {
        let parsed = parse(
            "a.com, 1, DIRECT\nA.com, 1, direct, cert\na.com, 1, RESELLER\na.com, 2, DIRECT\n",
        );
        let records: Vec<(u32, &str, Relationship)> = parsed
            .records
            .iter()
            .map(|record| (record.line, record.account_id.as_str(), record.relationship))
            .collect();
        assert_eq!(
            records,
            [
                (1, "1", Relationship::Direct),
                (3, "1", Relationship::Reseller),
                (4, "2", Relationship::Direct),
            ]
        );
        assert!(parsed.invalid.is_empty());
    }

    #[test]
    fn records_are_looked_up_by_any_of_an_ad_systems_domains() {
        let parsed = parse("xandr.com, 1, DIRECT\nappnexus.com, 2, RESELLER\nopenx.com, 3, DIRECT");
        let ids: Vec<&str> = parsed
            .records_for(&["appnexus.com", "xandr.com"])
            .map(|record| record.account_id.as_str())
            .collect();
        assert_eq!(ids, ["1", "2"]);
        assert_eq!(parsed.records_for(&["criteo.com"]).count(), 0);
    }

    #[test]
    fn long_lines_are_stored_as_short_excerpts() {
        let parsed = parse(&format!(
            "{}, 1, DIRECT\n{}",
            "a".repeat(400),
            "x".repeat(900)
        ));
        assert!(parsed.records.is_empty());
        assert!(
            parsed
                .invalid
                .iter()
                .all(|line| line.text.chars().count() <= 300)
        );
        assert_eq!(parsed.invalid.len(), 2);
    }

    #[test]
    fn http_answers_are_read_conservatively() {
        let file = b"appnexus.com, 958, DIRECT\n";
        let found = |outcome: AdsTxtOutcome| matches!(outcome, AdsTxtOutcome::Found(parsed) if parsed.records.len() == 1);
        assert!(found(read_response(
            200,
            Some("text/plain; charset=utf-8"),
            file
        )));
        assert!(found(read_response(
            200,
            Some("application/octet-stream"),
            file
        )));
        assert!(found(read_response(200, None, file)));

        assert_eq!(
            read_response(404, Some("text/html"), b"<html>"),
            AdsTxtOutcome::Missing
        );
        assert_eq!(read_response(410, None, b""), AdsTxtOutcome::Missing);
        for status in [401, 403, 429, 500, 503] {
            assert_eq!(
                read_response(status, Some("text/plain"), file),
                AdsTxtOutcome::Unreadable(format!("the site answered HTTP {status}")),
            );
        }
        let web_page = AdsTxtOutcome::Unreadable(
            "the site answered with a web page instead of a text file".into(),
        );
        assert_eq!(
            read_response(200, Some("TEXT/HTML; charset=utf-8"), file),
            web_page
        );
        assert_eq!(
            read_response(
                200,
                Some("text/plain"),
                b"\n <!DOCTYPE html><html><body>Not found"
            ),
            web_page
        );
        // Error pages and soft 404s that do not announce themselves as HTML.
        for page in [
            "<head><title>404 Not Found</title></head>",
            "\u{feff}  <script>location.href='/'</script>",
            "<svg xmlns='http://www.w3.org/2000/svg'/>",
        ] {
            assert_eq!(
                read_response(200, Some("text/plain"), page.as_bytes()),
                web_page,
                "{page}"
            );
        }
        assert_eq!(
            read_response(
                200,
                None,
                b"Page not found\nThe page you requested, /ads.txt, does not exist.\n"
            ),
            AdsTxtOutcome::Unreadable(
                "none of the lines in the answer is an ads.txt record".into()
            )
        );
        for no_records in ["", "# nothing yet\n", "CONTACT=ads@example.com\n"] {
            assert_eq!(
                read_response(200, Some("text/plain"), no_records.as_bytes()),
                AdsTxtOutcome::Unreadable("the file has no seller records".into()),
                "{no_records:?}"
            );
        }
        // The specification's way of saying that nobody is authorised.
        assert!(found(read_response(
            200,
            Some("text/plain"),
            b"placeholder.example.com, placeholder, DIRECT, placeholder\n"
        )));
        let not_text =
            AdsTxtOutcome::Unreadable("the site answered with data that is not text".into());
        assert_eq!(
            read_response(200, Some("text/plain"), b"\x1f\x8b\x08\x00compressed"),
            not_text
        );
        assert_eq!(
            read_response(
                200,
                Some("text/plain"),
                b"appnexus.com, 1, DIRECT\n\x00\x01"
            ),
            not_text
        );
    }
}
