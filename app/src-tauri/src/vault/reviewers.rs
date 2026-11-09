//! Reviewers: the people whose feedback the Author records as Comments. They belong
//! to the Vault: each is a `reviewer/<id>` record in `vault.db` with a name, a role label, a
//! colour and whether they've been removed. Removing one only stops them being offered as a
//! sender; their Comments keep their name and colour.

use super::*;
use wrtier::change::reviewer_record;

/// The colour slots, in the order new Reviewers get them. Every theme defines each one (a
/// strong colour for names and a soft one for highlights), so they stay readable in light and
/// dark, and a CSS theme in `themes/` can change them.
pub const COLOURS: &[&str] = &["teal", "amber", "red", "violet", "green", "orange", "pink", "brown"];

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Reviewer {
    /// Random, made when the Reviewer is added.
    pub id: String,
    pub name: String,
    /// Such as "editor", "beta reader" or "AI"; may be empty.
    pub role: String,
    /// One of `COLOURS`.
    pub colour: String,
    pub removed: bool,
}

/// A Reviewer's ID: 8 random hex digits, like a Device ID.
fn new_reviewer_id() -> Result<String, String> {
    let mut bytes = [0u8; 4];
    getrandom::fill(&mut bytes).map_err(|e| format!("couldn't make a Reviewer ID: {e}"))?;
    Ok(format!("{:08x}", u32::from_le_bytes(bytes)))
}

fn check_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err("A Reviewer's name needs 1 to 100 characters.".into());
    }
    Ok(name.to_string())
}

fn check_role(role: &str) -> Result<String, String> {
    let role = role.trim();
    if role.chars().count() > 60 {
        return Err("A role label can have at most 60 characters.".into());
    }
    Ok(role.to_string())
}

fn check_colour(colour: &str) -> Result<String, String> {
    if COLOURS.contains(&colour) { Ok(colour.to_string()) } else { Err(format!("Not a Reviewer colour: {colour}")) }
}

impl Vault {
    /// Every Reviewer, removed ones too, by name. `vault.db` is read again first if another
    /// Device changed it.
    pub fn reviewers(&mut self) -> Vec<Reviewer> {
        self.absorb_vault_copies(&mut Vec::new()); // any left are reported when the Works are listed
        let mut list: Vec<Reviewer> = self.vault_doc.records("reviewer/").into_iter()
            .filter_map(|record| self.reviewer(record.strip_prefix("reviewer/")?))
            .collect();
        list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.id.cmp(&b.id)));
        list
    }

    /// One Reviewer, if `vault.db` knows them.
    pub(crate) fn reviewer(&self, id: &str) -> Option<Reviewer> {
        let record = reviewer_record(id);
        let field = |name: &str| self.vault_doc.field(&record, name);
        Some(Reviewer {
            id: id.to_string(),
            name: field("name")?.to_string(),
            role: field("role").unwrap_or("").to_string(),
            colour: field("colour").filter(|c| COLOURS.contains(c)).unwrap_or(COLOURS[0]).to_string(),
            removed: field("removed") == Some("yes"),
        })
    }

    /// The colour a new Reviewer gets: the first nobody has (removed Reviewers count, since
    /// their Comments keep their colour), or else the one fewest have.
    fn free_colour(&mut self) -> &'static str {
        let taken: Vec<String> = self.reviewers().into_iter().map(|r| r.colour).collect();
        let uses = |colour: &&str| taken.iter().filter(|t| t == colour).count();
        COLOURS.iter().min_by_key(|c| uses(c)).copied().expect("there are colours")
    }

    /// Adds a Reviewer to the Vault, with the next free colour.
    pub fn add_reviewer(&mut self, name: &str, role: &str, now_ms: u64) -> Result<Reviewer, String> {
        let (name, role) = (check_name(name)?, check_role(role)?);
        let colour = self.free_colour();
        let id = new_reviewer_id()?;
        let record = reviewer_record(&id);
        let mut changes = vec![
            self.vault_doc.set_field_local(&record, "name", &name, now_ms),
            self.vault_doc.set_field_local(&record, "colour", colour, now_ms),
        ];
        if !role.is_empty() {
            changes.push(self.vault_doc.set_field_local(&record, "role", &role, now_ms));
        }
        self.save_vault(&changes)?;
        self.reviewer(&id).ok_or_else(|| "the new Reviewer wasn't recorded".into())
    }

    /// Changes a Reviewer's name, role and colour. Only what changed is recorded.
    pub fn update_reviewer(&mut self, id: &str, name: &str, role: &str, colour: &str, now_ms: u64) -> Result<Reviewer, String> {
        let current = self.reviewer(id).ok_or("No such Reviewer.")?;
        let record = reviewer_record(id);
        let mut changes = Vec::new();
        for (field, old, new) in [("name", current.name, check_name(name)?), ("role", current.role, check_role(role)?), ("colour", current.colour, check_colour(colour)?)] {
            if old != new {
                changes.push(self.vault_doc.set_field_local(&record, field, &new, now_ms));
            }
        }
        if !changes.is_empty() {
            self.save_vault(&changes)?;
        }
        self.reviewer(id).ok_or_else(|| "No such Reviewer.".into())
    }

    /// Removes a Reviewer (they're no longer offered as a sender), or brings one back.
    pub fn set_reviewer_removed(&mut self, id: &str, removed: bool, now_ms: u64) -> Result<Reviewer, String> {
        let current = self.reviewer(id).ok_or("No such Reviewer.")?;
        if current.removed != removed {
            let change = self.vault_doc.set_field_local(&reviewer_record(id), "removed", if removed { "yes" } else { "no" }, now_ms);
            self.save_vault(&[change])?;
        }
        self.reviewer(id).ok_or_else(|| "No such Reviewer.".into())
    }

    /// How a Comment's sender shows in a Work: their name and colour slot. The Author: the
    /// Preamble's author name (or "Author"), in the accent colour ("author"). A Reviewer: as
    /// `vault.db` has them now; one it doesn't know, by the name the Work recorded, in grey
    /// ("unknown").
    pub fn sender_label(&self, doc: &Doc, sender: &str) -> (String, String) {
        let Some(id) = sender.strip_prefix("reviewer/") else {
            let author = crate::preamble::Preamble::of(doc).author;
            let name = if author.trim().is_empty() { "Author".to_string() } else { author };
            return (name, "author".into());
        };
        match self.reviewer(id) {
            Some(r) => (r.name, r.colour),
            None => (doc.field(sender, "name").unwrap_or("Unknown reviewer").to_string(), "unknown".into()),
        }
    }

    /// Just before a Comment or Reply from `sender` is recorded in a Work: records the
    /// Reviewer's current name in it, if the Work has none for them or an older one, so the
    /// Work's `.db` reads on its own. The Author, or a Reviewer the Vault doesn't know, records
    /// nothing. The caller saves the Change.
    pub fn record_sender_name(&self, doc: &mut Doc, sender: &str, now_ms: u64) -> Option<Change> {
        let id = sender.strip_prefix("reviewer/")?;
        let name = self.reviewer(id)?.name;
        if doc.field(sender, "name") == Some(name.as_str()) {
            return None;
        }
        Some(doc.set_field_local(sender, "name", &name, now_ms))
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests_support::*;
    use super::*;
    use wrtier::change::AUTHOR;

    #[test]
    fn a_reviewer_gets_the_next_free_colour_and_can_be_changed() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        let anna = vault.add_reviewer("  Anna ", "editor", 1000).unwrap();
        assert_eq!((anna.name.as_str(), anna.role.as_str(), anna.colour.as_str(), anna.removed), ("Anna", "editor", "teal", false));
        let ben = vault.add_reviewer("Ben", "", 2000).unwrap();
        assert_eq!(ben.colour, "amber");
        let ben = vault.update_reviewer(&ben.id, "Benjamin", "beta reader", "violet", 3000).unwrap();
        assert_eq!((ben.name.as_str(), ben.role.as_str(), ben.colour.as_str()), ("Benjamin", "beta reader", "violet"));
        // The next one gets the first colour nobody has: amber is free again.
        assert_eq!(vault.add_reviewer("Cleo", "AI", 4000).unwrap().colour, "amber");
        let names: Vec<String> = vault.reviewers().into_iter().map(|r| r.name).collect();
        assert_eq!(names, ["Anna", "Benjamin", "Cleo"]);
        assert!(vault.add_reviewer(" ", "", 5000).is_err());
        assert!(vault.update_reviewer(&anna.id, "Anna", "", "purple", 5000).is_err());
    }

    #[test]
    fn colours_are_reused_once_all_are_taken() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        for (i, colour) in COLOURS.iter().enumerate() {
            assert_eq!(vault.add_reviewer(&format!("R{i}"), "", 1000).unwrap().colour, *colour);
        }
        assert_eq!(vault.add_reviewer("Ninth", "", 2000).unwrap().colour, COLOURS[0]);
        assert_eq!(vault.add_reviewer("Tenth", "", 3000).unwrap().colour, COLOURS[1]);
    }

    #[test]
    fn a_reviewer_added_on_one_device_shows_on_another_and_removing_keeps_them() {
        let dir = tempfile::tempdir().unwrap();
        let mut laptop = open(dir.path());
        let anna = laptop.add_reviewer("Anna", "editor", 1000).unwrap();
        // The phone opens the same (synced) Vault.
        let Opened::Vault(mut phone) = Vault::open(dir.path(), DeviceId(2), false, &dir.path().join(".recovery2")).unwrap() else { panic!() };
        assert_eq!(phone.reviewers(), [anna.clone()]);
        // Removed on the phone: the laptop sees it the next time it looks, with the name and colour kept.
        phone.set_reviewer_removed(&anna.id, true, 2000).unwrap();
        let seen = laptop.reviewers();
        assert_eq!(seen.len(), 1);
        assert_eq!((seen[0].name.as_str(), seen[0].colour.as_str(), seen[0].removed), ("Anna", "teal", true));
        // A removed Reviewer still counts for colours, and can be brought back.
        assert_eq!(laptop.add_reviewer("Ben", "", 3000).unwrap().colour, "amber");
        assert!(!laptop.set_reviewer_removed(&anna.id, false, 4000).unwrap().removed);
    }

    /// Writes a Work with Comments into the Vault named by WRTIER_SAMPLE_VAULT, to look at the
    /// highlights before the app can add Comments itself:
    /// `WRTIER_SAMPLE_VAULT=C:\Writing cargo test -p wrtier_app write_a_commented_sample -- --ignored`
    #[test]
    #[ignore]
    fn write_a_commented_sample() {
        let root = PathBuf::from(std::env::var("WRTIER_SAMPLE_VAULT").expect("set WRTIER_SAMPLE_VAULT to a Vault's folder"));
        let Opened::Vault(mut vault) = Vault::open(&root, DeviceId(0x5a3b1e), true, &std::env::temp_dir().join("wrtier-sample-recovery")).unwrap() else { panic!() };
        let now = 1_700_000_000_000;
        let anna = vault.add_reviewer("Anna", "editor", now).unwrap();
        let ben = vault.add_reviewer("Ben", "beta reader", now).unwrap();
        let (anna, ben) = (reviewer_record(&anna.id), reviewer_record(&ben.id));
        let mut doc = Doc::new(DeviceId(0x5a3b1e));
        let text = "= Rain\n\nThe rain had stopped. Nobody on the street said a word, and the lamps came on one by one.\n\n\
                    She folded the letter twice and put it in her coat. Later she would read it again, slowly.\n\n\
                    This sentence is about to be deleted.\n";
        let mut changes = vec![doc.insert_local(0, text, now)];
        let at = |doc: &Doc, s: &str| doc.text().find(s).map(|b| doc.text()[..b].chars().count()).unwrap();
        let count = |s: &str| s.chars().count();
        for sender in [&anna, &ben] {
            changes.extend(vault.record_sender_name(&mut doc, sender, now));
        }
        let mut comment = |doc: &mut Doc, quote: &str, sender: &str, body: &str| {
            let change = doc.comment_local(at(doc, quote), count(quote), sender, body, now);
            changes.push(change.clone());
            change.id
        };
        comment(&mut doc, "The rain had stopped.", &anna, "Too abrupt as an opening?");
        comment(&mut doc, "Nobody on the street said a word", &ben, "I love this line.");
        comment(&mut doc, "street said a word, and the lamps", AUTHOR, "Check the lamps: gas or electric in 1910?");
        let resolved = comment(&mut doc, "folded the letter twice", &anna, "Which letter? Name it earlier.");
        comment(&mut doc, "Later she would read it again, slowly.", &ben, "Split by a Move: shows in two places.");
        comment(&mut doc, "This sentence is about to be deleted.", &anna, "Detached: its text is gone.");
        changes.push(doc.resolve_comment_local(resolved, true, now));
        let from = at(&doc, "slowly.");
        changes.push(doc.move_local(from, count("slowly."), at(&doc, "= Rain") + count("= Rain\n"), now));
        let gone = at(&doc, "This sentence");
        changes.push(doc.delete_local(gone, count("This sentence is about to be deleted."), now));
        let db = root.join("Commented sample.db");
        storage::save(&db, &changes, &doc).unwrap();
        println!("Wrote {}", db.display());
    }

    #[test]
    fn a_reviewers_comment_records_their_name_and_shows_their_colour() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Rain.typ"), "The rain had stopped.").unwrap();
        let mut vault = open(dir.path());
        let anna = vault.add_reviewer("Anna", "editor", 1000).unwrap();
        let sender = reviewer_record(&anna.id);
        vault.open_work("Rain", 1000).unwrap();
        let op = crate::work::Op::Comment { from: 4, to: 8, sender: sender.clone(), body: "too wet?".into(), sender_name: None };
        let state = vault.edit("Rain", vec![op, crate::work::Op::Marks], 2000).unwrap();
        let mark = state.marks.unwrap().remove(0);
        assert_eq!((mark.name.as_str(), mark.colour.as_str()), ("Anna", "teal"));
        assert_eq!(vault.works["Rain"].doc.field(&sender, "name"), Some("Anna"));
    }

    #[test]
    fn a_sender_shows_by_their_current_name_and_colour() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        let anna = vault.add_reviewer("Anna", "", 1000).unwrap();
        let sender = reviewer_record(&anna.id);
        let mut doc = Doc::new(ME);
        assert_eq!(vault.sender_label(&doc, AUTHOR), ("Author".into(), "author".into()));
        doc.set_field_local("preamble", "author", "R. Fiehn", 1000);
        assert_eq!(vault.sender_label(&doc, AUTHOR), ("R. Fiehn".into(), "author".into()));
        assert_eq!(vault.sender_label(&doc, &sender), ("Anna".into(), "teal".into()));
        // A Reviewer this Vault doesn't know (the Work came from elsewhere): the name the Work recorded.
        doc.set_field_local("reviewer/0badf00d", "name", "Bo", 2000);
        assert_eq!(vault.sender_label(&doc, "reviewer/0badf00d"), ("Bo".into(), "unknown".into()));
        assert_eq!(vault.sender_label(&doc, "reviewer/12345678"), ("Unknown reviewer".into(), "unknown".into()));
    }

    #[test]
    fn a_work_records_a_reviewers_current_name_before_their_comment() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        let anna = vault.add_reviewer("Anna", "", 1000).unwrap();
        let sender = reviewer_record(&anna.id);
        let mut doc = Doc::new(ME);
        doc.insert_local(0, "The rain had stopped.", 1000);
        let recorded = vault.record_sender_name(&mut doc, &sender, 2000);
        assert!(recorded.is_some());
        assert_eq!(doc.field(&sender, "name"), Some("Anna"));
        // Not again while the name is the same; again once it changes.
        assert!(vault.record_sender_name(&mut doc, &sender, 3000).is_none());
        vault.update_reviewer(&anna.id, "Anna K.", "", "teal", 4000).unwrap();
        vault.record_sender_name(&mut doc, &sender, 5000).unwrap();
        assert_eq!(doc.field(&sender, "name"), Some("Anna K."));
        // The Author, or a Reviewer this Vault doesn't know, records nothing.
        assert!(vault.record_sender_name(&mut doc, AUTHOR, 6000).is_none());
        assert!(vault.record_sender_name(&mut doc, "reviewer/00000000", 6000).is_none());
        // It's not the Work's content: returning to a Milestone wouldn't touch it.
        assert!(wrtier::change::is_meta_record(&sender));
    }
}
