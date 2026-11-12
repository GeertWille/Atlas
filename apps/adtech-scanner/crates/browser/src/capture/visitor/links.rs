//! Which link a scripted visitor follows. The page supplies facts about its
//! links (`injected/links.js`); every decision is made here. The policy errs
//! towards skipping: a visit that follows nothing is fine, one that logs the
//! reader out, buys something or leaves the site is not.

use std::collections::{BTreeMap, HashSet};

use serde::Deserialize;
use url::Url;

use super::rng::Rng;
use super::site::Site;
use super::{Rect, Size};

/// How many of the best links the choice is made among.
pub(crate) const SHORTLIST: usize = 5;

/// One `<a href>` of the page as the page describes it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LinkFacts {
    /// The address the browser would navigate to.
    pub href: String,
    pub text: String,
    pub rel: String,
    pub target: String,
    pub download: bool,
    pub role: String,
    /// Relative to the viewport.
    pub rect: Rect,
    pub visible: bool,
    pub region: Region,
    pub in_ad: bool,
    pub in_form: bool,
    pub has_image: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Region {
    Content,
    Nav,
    Aside,
    Footer,
    Other,
}

/// Why a link is never followed. The order is the order the checks run in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Skip {
    Hidden,
    InsideAd,
    Scheme,
    Anchor,
    OffSite,
    File,
    Rel,
    Sensitive,
    Control,
    Visited,
}

impl Skip {
    /// The name the reason has in the audit trail.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Skip::Hidden => "hidden",
            Skip::InsideAd => "inside-ad",
            Skip::Scheme => "scheme",
            Skip::Anchor => "anchor",
            Skip::OffSite => "off-site",
            Skip::File => "file",
            Skip::Rel => "rel",
            Skip::Sensitive => "sensitive",
            Skip::Control => "control",
            Skip::Visited => "visited",
        }
    }
}

/// Where the visit is when it looks for a link.
pub(crate) struct Surroundings<'a> {
    pub current: &'a Url,
    pub site: &'a Site,
    /// Pages already visited or tried, as [`page_key`]s.
    pub visited: &'a HashSet<String>,
    pub viewport: Size,
}

/// A link the policy allows, with what speaks for and against it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Followable {
    /// Position in the facts the page supplied.
    pub index: usize,
    pub url: Url,
    pub score: i32,
    pub reasons: Vec<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Survey {
    /// The best links, best first; at most [`SHORTLIST`].
    pub shortlist: Vec<Followable>,
    /// Position of the chosen link in `shortlist`.
    pub chosen: Option<usize>,
    pub skipped: BTreeMap<Skip, usize>,
}

/// Looks at every link of the page and picks the one to follow, if any.
pub(crate) fn survey(rng: &mut Rng, links: &[LinkFacts], surroundings: &Surroundings) -> Survey {
    let mut skipped = BTreeMap::new();
    let mut followable = Vec::new();
    let mut seen = HashSet::new();
    for (index, link) in links.iter().enumerate() {
        match classify(link, surroundings) {
            Err(skip) => *skipped.entry(skip).or_default() += 1,
            // A page links its lead story several times; it is one candidate.
            Ok(url) if !seen.insert(page_key(&url)) => {}
            Ok(url) => {
                let (score, reasons) = score(link, &url, surroundings.viewport);
                followable.push(Followable {
                    index,
                    url,
                    score,
                    reasons,
                });
            }
        }
    }
    // Stable: equally good links stay in document order.
    followable.sort_by_key(|link| std::cmp::Reverse(link.score));
    followable.truncate(SHORTLIST);
    let floor = followable.last().map_or(0, |link| link.score);
    let weights: Vec<f64> = followable
        .iter()
        .map(|link| f64::from(link.score - floor + 1))
        .collect();
    Survey {
        chosen: rng.weighted(&weights),
        shortlist: followable,
        skipped,
    }
}

/// The address to follow, or the first reason not to.
pub(crate) fn classify(link: &LinkFacts, surroundings: &Surroundings) -> Result<Url, Skip> {
    if !link.visible {
        return Err(Skip::Hidden);
    }
    if link.in_ad {
        return Err(Skip::InsideAd);
    }
    let url = Url::parse(&link.href).map_err(|_| Skip::Scheme)?;
    classify_address(&url, surroundings)?;
    if link.download || has_file_extension(&url) {
        return Err(Skip::File);
    }
    let rel = words(&link.rel);
    if ["nofollow", "sponsored", "ugc"]
        .iter()
        .any(|value| rel.iter().any(|word| word == value))
    {
        return Err(Skip::Rel);
    }
    if is_sensitive(&url, &link.text) {
        return Err(Skip::Sensitive);
    }
    if link.in_form || link.role.eq_ignore_ascii_case("button") {
        return Err(Skip::Control);
    }
    if surroundings.visited.contains(&page_key(&url)) {
        return Err(Skip::Visited);
    }
    Ok(url)
}

/// The checks that need nothing but the address. They run again on the
/// address a link has at the moment it is clicked.
pub(crate) fn classify_address(url: &Url, surroundings: &Surroundings) -> Result<(), Skip> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(Skip::Scheme);
    }
    if page_key(url) == page_key(surroundings.current) {
        return Err(Skip::Anchor);
    }
    if !surroundings.site.contains(url) {
        return Err(Skip::OffSite);
    }
    Ok(())
}

/// What makes two addresses the same page: everything but the fragment.
pub(crate) fn page_key(url: &Url) -> String {
    let mut page = url.clone();
    page.set_fragment(None);
    page.into()
}

/// An address as it is written into logs and reports: no query, whose values
/// can identify a reader, and no fragment.
pub(crate) fn loggable(url: &Url) -> String {
    format!("{}{}", url.origin().ascii_serialization(), url.path())
}

const FILE_EXTENSIONS: &[&str] = &[
    "pdf", "zip", "gz", "tar", "rar", "7z", "dmg", "pkg", "exe", "msi", "apk", "doc", "docx",
    "xls", "xlsx", "ppt", "pptx", "csv", "txt", "mp3", "wav", "mp4", "mov", "avi", "webm", "jpg",
    "jpeg", "png", "gif", "webp", "svg", "xml", "rss", "atom", "json", "ics",
];

fn has_file_extension(url: &Url) -> bool {
    let last_segment = url.path().rsplit('/').next().unwrap_or_default();
    last_segment.rsplit_once('.').is_some_and(|(_, extension)| {
        FILE_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
    })
}

/// Words that name something a visitor does to an account, a basket or the
/// site's data, in English, German, French, Spanish, Italian and Dutch.
const SENSITIVE_WORDS: &[&str] = &[
    "login",
    "signin",
    "signup",
    "signout",
    "register",
    "account",
    "profile",
    "cart",
    "basket",
    "checkout",
    "payment",
    "buy",
    "order",
    "subscribe",
    "unsubscribe",
    "logout",
    "delete",
    "remove",
    "cancel",
    "admin",
    "auth",
    "password",
    "donate",
    "download",
    "print",
    "share",
    "vote",
    "report",
    "anmelden",
    "einloggen",
    "abmelden",
    "registrieren",
    "konto",
    "warenkorb",
    "kasse",
    "kaufen",
    "bestellen",
    "abonnieren",
    "abbestellen",
    "löschen",
    "loeschen",
    "entfernen",
    "abbrechen",
    "passwort",
    "spenden",
    "herunterladen",
    "drucken",
    "teilen",
    "melden",
    "connexion",
    "déconnexion",
    "deconnexion",
    "inscription",
    "compte",
    "panier",
    "paiement",
    "acheter",
    "commander",
    "abonner",
    "désabonner",
    "desabonner",
    "supprimer",
    "annuler",
    "télécharger",
    "telecharger",
    "imprimer",
    "partager",
    "signaler",
    "acceder",
    "registro",
    "registrarse",
    "cuenta",
    "carrito",
    "pago",
    "comprar",
    "pedido",
    "suscribirse",
    "eliminar",
    "borrar",
    "cancelar",
    "contraseña",
    "contrasena",
    "donar",
    "descargar",
    "imprimir",
    "compartir",
    "votar",
    "denunciar",
    "accedi",
    "esci",
    "registrati",
    "carrello",
    "cassa",
    "pagamento",
    "acquista",
    "ordina",
    "abbonati",
    "elimina",
    "rimuovi",
    "annulla",
    "dona",
    "scarica",
    "stampa",
    "condividi",
    "vota",
    "segnala",
    "inloggen",
    "uitloggen",
    "aanmelden",
    "afmelden",
    "registreren",
    "winkelwagen",
    "winkelmand",
    "afrekenen",
    "betalen",
    "kopen",
    "abonneren",
    "uitschrijven",
    "verwijderen",
    "annuleren",
    "wachtwoord",
    "doneren",
    "downloaden",
    "afdrukken",
    "delen",
    "stemmen",
];

/// Query keys that carry a credential or make the request do something.
const SENSITIVE_QUERY_KEYS: &[&str] = &["token", "key", "session", "sid", "nonce", "action"];

/// A headline may contain "report" or "order"; a control is named by it.
/// Longer phrases than this are taken for headlines and article slugs.
const MAX_CONTROL_WORDS: usize = 3;

fn is_sensitive(url: &Url, text: &str) -> bool {
    let path_names_an_action = url
        .path_segments()
        .into_iter()
        .flatten()
        .any(|segment| names_an_action(&words(segment)));
    let query_names_an_action = url.query_pairs().any(|(key, _)| {
        let key = key.to_lowercase();
        SENSITIVE_QUERY_KEYS.contains(&key.as_str()) || names_an_action(&words(&key))
    });
    path_names_an_action || query_names_an_action || names_an_action(&words(text))
}

fn names_an_action(phrase: &[String]) -> bool {
    if phrase.len() > MAX_CONTROL_WORDS {
        return false;
    }
    let is_sensitive = |word: &str| SENSITIVE_WORDS.contains(&word);
    // "log in", "sign-up" and "check out" are written as two words.
    phrase.iter().any(|word| is_sensitive(word))
        || phrase
            .windows(2)
            .any(|pair| is_sensitive(&format!("{}{}", pair[0], pair[1])))
}

/// Lower-case words of a path segment, an attribute or a link text.
fn words(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Pages every site has and no reader came for.
const BOILERPLATE_WORDS: &[&str] = &[
    "privacy",
    "terms",
    "imprint",
    "impressum",
    "contact",
    "about",
    "advertise",
    "careers",
    "cookies",
    "legal",
];

fn score(link: &LinkFacts, url: &Url, viewport: Size) -> (i32, Vec<&'static str>) {
    let mut score = 0;
    let mut reasons = Vec::new();
    let mut count = |points: i32, reason: &'static str| {
        score += points;
        reasons.push(reason);
    };
    match link.region {
        Region::Content => count(3, "in the main content"),
        Region::Nav => count(2, "in the navigation"),
        Region::Aside => count(1, "in a side column"),
        Region::Footer => count(-2, "in the footer"),
        Region::Other => {}
    }
    if has_article_like_path(url) {
        count(2, "article-like path");
    }
    if (25..=120).contains(&link.text.chars().count()) {
        count(1, "headline-length text");
    }
    if link.has_image {
        count(1, "has an image");
    }
    let near = -viewport.height..=2.0 * viewport.height;
    if near.contains(&link.rect.y) {
        count(1, "near the reader's position");
    }
    if link.target.eq_ignore_ascii_case("_blank") {
        count(-1, "opens a new tab");
    }
    if url.query().is_some() {
        count(-1, "has a query string");
    }
    let path_and_text = format!("{} {}", url.path(), link.text);
    if words(&path_and_text)
        .iter()
        .any(|word| BOILERPLATE_WORDS.contains(&word.as_str()))
    {
        count(-5, "boilerplate page");
    }
    (score, reasons)
}

fn has_article_like_path(url: &Url) -> bool {
    let segments: Vec<&str> = url
        .path_segments()
        .into_iter()
        .flatten()
        .filter(|segment| !segment.is_empty())
        .collect();
    let is_year =
        |segment: &&str| segment.len() == 4 && segment.parse::<u32>().is_ok_and(|year| year > 1990);
    segments.len() >= 2
        || segments.iter().any(|segment| words(segment).len() >= 3)
        || segments.iter().any(is_year)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEWPORT: Size = Size {
        width: 1440.0,
        height: 900.0,
    };

    fn link(href: &str) -> LinkFacts {
        LinkFacts {
            href: href.into(),
            text: "The village that bought its own substation".into(),
            rel: String::new(),
            target: String::new(),
            download: false,
            role: String::new(),
            rect: Rect {
                x: 100.0,
                y: 400.0,
                width: 300.0,
                height: 24.0,
            },
            visible: true,
            region: Region::Content,
            in_ad: false,
            in_form: false,
            has_image: false,
        }
    }

    struct Visit {
        current: Url,
        site: Site,
        visited: HashSet<String>,
    }

    impl Visit {
        fn at(current: &str) -> Self {
            let current = Url::parse(current).unwrap();
            Self {
                site: Site::of(&current).unwrap(),
                visited: HashSet::from([page_key(&current)]),
                current,
            }
        }

        fn surroundings(&self) -> Surroundings<'_> {
            Surroundings {
                current: &self.current,
                site: &self.site,
                visited: &self.visited,
                viewport: VIEWPORT,
            }
        }

        fn verdict(&self, link: &LinkFacts) -> Result<String, Skip> {
            classify(link, &self.surroundings()).map(String::from)
        }
    }

    #[test]
    fn every_skip_reason_has_a_link_that_meets_it() {
        let mut visit = Visit::at("https://news.example/climate/tide-town?ref=home");
        visit
            .visited
            .insert("https://news.example/climate/read-before".into());
        let with = |change: fn(&mut LinkFacts)| {
            let mut link = link("https://news.example/climate/kelp-carbon");
            change(&mut link);
            link
        };
        let cases: Vec<(LinkFacts, Skip)> = vec![
            (with(|link| link.visible = false), Skip::Hidden),
            (with(|link| link.in_ad = true), Skip::InsideAd),
            (link("mailto:desk@news.example"), Skip::Scheme),
            (link("tel:+3112345678"), Skip::Scheme),
            (link("javascript:void(0)"), Skip::Scheme),
            (link("data:text/html,hi"), Skip::Scheme),
            (link("blob:https://news.example/9f"), Skip::Scheme),
            (link("not an address"), Skip::Scheme),
            (
                link("https://news.example/climate/tide-town?ref=home#comments"),
                Skip::Anchor,
            ),
            (link("https://partner.example/offer"), Skip::OffSite),
            (link("http://news.example/climate/kelp"), Skip::OffSite),
            (link("https://news.example/files/media-kit.PDF"), Skip::File),
            (link("https://news.example/feed.rss"), Skip::File),
            (with(|link| link.download = true), Skip::File),
            (
                with(|link| link.rel = "noopener NoFollow".into()),
                Skip::Rel,
            ),
            (with(|link| link.rel = "sponsored".into()), Skip::Rel),
            (with(|link| link.rel = "ugc".into()), Skip::Rel),
            (link("https://news.example/account/login"), Skip::Sensitive),
            (link("https://news.example/logout"), Skip::Sensitive),
            (link("https://news.example/sign-up"), Skip::Sensitive),
            (link("https://news.example/my/Warenkorb"), Skip::Sensitive),
            (
                link("https://news.example/climate/kelp?session=abc"),
                Skip::Sensitive,
            ),
            (
                link("https://news.example/climate/kelp?do_delete=1"),
                Skip::Sensitive,
            ),
            (with(|link| link.text = "Log in".into()), Skip::Sensitive),
            (with(|link| link.text = "Subscribe".into()), Skip::Sensitive),
            (
                with(|link| link.text = "Se déconnexion".into()),
                Skip::Sensitive,
            ),
            (with(|link| link.in_form = true), Skip::Control),
            (with(|link| link.role = "button".into()), Skip::Control),
            (
                link("https://news.example/climate/read-before#top"),
                Skip::Visited,
            ),
        ];
        for (link, expected) in cases {
            assert_eq!(
                visit.verdict(&link),
                Err(expected),
                "{} / {:?}",
                link.href,
                link.text
            );
        }
    }

    #[test]
    fn the_first_reason_in_the_order_of_the_checks_is_the_one_reported() {
        let visit = Visit::at("https://news.example/");
        let mut everything_wrong = link("https://partner.example/account/login.pdf");
        everything_wrong.rel = "sponsored".into();
        everything_wrong.in_form = true;
        assert_eq!(visit.verdict(&everything_wrong), Err(Skip::OffSite));
        everything_wrong.in_ad = true;
        assert_eq!(visit.verdict(&everything_wrong), Err(Skip::InsideAd));
        everything_wrong.visible = false;
        assert_eq!(visit.verdict(&everything_wrong), Err(Skip::Hidden));
    }

    #[test]
    fn headlines_and_slugs_may_use_words_that_name_actions_elsewhere() {
        let visit = Visit::at("https://news.example/");
        let mut headline = link("https://news.example/2026/court-order-blocks-harbour-plan");
        headline.text = "Report: court order blocks the harbour plan".into();
        assert_eq!(
            visit.verdict(&headline).unwrap(),
            "https://news.example/2026/court-order-blocks-harbour-plan"
        );
        let other_subdomain = link("https://sport.news.example/cup/final");
        assert!(visit.verdict(&other_subdomain).is_ok());
    }

    #[test]
    fn links_are_ranked_by_where_they_are_and_what_they_look_like() {
        let visit = Visit::at("https://news.example/");
        let score_of = |link: &LinkFacts| {
            let url = Url::parse(&link.href).unwrap();
            score(link, &url, VIEWPORT).0
        };
        let article = link("https://news.example/climate/kelp-carbon");
        assert_eq!(score_of(&article), 3 + 2 + 1 + 1);

        let mut with_image = article.clone();
        with_image.has_image = true;
        assert_eq!(score_of(&with_image), score_of(&article) + 1);

        let mut in_nav = link("https://news.example/sport");
        in_nav.region = Region::Nav;
        in_nav.text = "Sport".into();
        assert_eq!(score_of(&in_nav), 2 + 1);

        let mut far_down = article.clone();
        far_down.rect.y = 5_000.0;
        assert_eq!(score_of(&far_down), score_of(&article) - 1);

        let mut new_tab = article.clone();
        new_tab.target = "_blank".into();
        assert_eq!(score_of(&new_tab), score_of(&article) - 1);

        let with_query = link("https://news.example/climate/kelp-carbon?page=2");
        assert_eq!(score_of(&with_query), score_of(&article) - 1);

        let mut privacy = link("https://news.example/privacy");
        privacy.region = Region::Footer;
        privacy.text = "Privacy".into();
        assert_eq!(score_of(&privacy), -2 + 1 - 5);

        let mut links = vec![privacy, in_nav, far_down, article.clone(), with_image];
        links[2].href = "https://news.example/climate/white-roofs".into();
        links[4].href = "https://news.example/climate/permit-queue".into();
        let survey = survey(&mut Rng::new(1), &links, &visit.surroundings());
        let order: Vec<usize> = survey.shortlist.iter().map(|link| link.index).collect();
        assert_eq!(order, [4, 3, 2, 1, 0]);
    }

    #[test]
    fn the_shortlist_is_the_best_five_without_duplicates_and_counts_what_was_skipped() {
        let visit = Visit::at("https://news.example/");
        let mut links: Vec<LinkFacts> = (0..8)
            .map(|n| link(&format!("https://news.example/climate/story-{n}")))
            .collect();
        links.push(link("https://news.example/climate/story-0#comments"));
        links.push(link("https://partner.example/"));
        links.push(link("https://news.example/logout"));
        links.push(link("mailto:desk@news.example"));
        links[7].has_image = true;

        let survey = survey(&mut Rng::new(5), &links, &visit.surroundings());

        assert_eq!(survey.shortlist.len(), SHORTLIST);
        let order: Vec<usize> = survey.shortlist.iter().map(|link| link.index).collect();
        assert_eq!(order, [7, 0, 1, 2, 3]);
        assert_eq!(
            survey.skipped,
            BTreeMap::from([(Skip::Scheme, 1), (Skip::OffSite, 1), (Skip::Sensitive, 1)])
        );
        assert!(survey.chosen.is_some_and(|chosen| chosen < SHORTLIST));
    }

    #[test]
    fn the_choice_is_decided_by_the_seed_and_favours_better_links() {
        let visit = Visit::at("https://news.example/");
        let mut links: Vec<LinkFacts> = (0..5)
            .map(|n| link(&format!("https://news.example/climate/story-{n}")))
            .collect();
        links[0].has_image = true;
        links[0].rect.y = 100.0;
        for weak in &mut links[1..] {
            weak.region = Region::Other;
        }
        let chosen = |seed| {
            survey(&mut Rng::new(seed), &links, &visit.surroundings())
                .chosen
                .unwrap()
        };
        assert_eq!(chosen(17), chosen(17));
        let picks: Vec<usize> = (0..200).map(chosen).collect();
        let best = picks.iter().filter(|pick| **pick == 0).count();
        assert!(best > 80, "the best link was chosen {best} times of 200");
        assert!(picks.iter().any(|pick| *pick != 0), "others get a turn");
    }

    #[test]
    fn a_page_without_a_followable_link_has_no_choice() {
        let visit = Visit::at("https://news.example/");
        let links = [
            link("https://partner.example/"),
            link("https://news.example/"),
        ];
        let survey = survey(&mut Rng::new(1), &links, &visit.surroundings());
        assert_eq!(survey.chosen, None);
        assert!(survey.shortlist.is_empty());
        assert_eq!(
            survey.skipped,
            BTreeMap::from([(Skip::Anchor, 1), (Skip::OffSite, 1)])
        );
    }

    #[test]
    fn addresses_are_logged_without_query_and_fragment() {
        let url = Url::parse("https://news.example:8443/a/b?user=42&utm=x#top").unwrap();
        assert_eq!(loggable(&url), "https://news.example:8443/a/b");
        assert_eq!(
            page_key(&url),
            "https://news.example:8443/a/b?user=42&utm=x"
        );
    }

    #[test]
    fn link_facts_are_read_from_what_the_page_script_returns() {
        let facts: LinkFacts = serde_json::from_str(
            r#"{"href":"https://news.example/a","text":"A","rel":"","target":"_blank",
                "download":false,"role":"","rect":{"x":1,"y":2,"width":30,"height":40},
                "visible":true,"region":"aside","inAd":false,"inForm":false,"hasImage":true}"#,
        )
        .unwrap();
        assert_eq!(facts.region, Region::Aside);
        assert_eq!(facts.rect.height, 40.0);
        assert!(facts.has_image);
    }
}
