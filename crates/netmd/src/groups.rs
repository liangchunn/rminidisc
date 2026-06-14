use log::debug;
use rusb::UsbContext;

use crate::{
    error::{NetMDError, Result},
    title::{get_half_width_title_length, half_width_to_full_width_range},
    types::{Disc, Encoding, Group, Track},
};

use super::NetMD;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawTrackGroup {
    pub name: Option<String>,
    pub full_width_name: Option<String>,
    pub tracks: Vec<u16>,
}

impl<T: UsbContext> NetMD<T> {
    /// Parses the disc's group structure. Mirrors
    /// `NetMDInterface.getTrackGroupList` (`netmd-interface.ts:509`).
    ///
    /// The first entry is the synthetic "ungrouped" bucket (`name: None`) holding
    /// any tracks not covered by a group, present only when such tracks exist.
    pub fn get_track_group_list(&self) -> Result<Vec<RawTrackGroup>> {
        debug!("get track group list");
        let raw_title = self.get_disc_title(false)?;
        let raw_full_title = self.get_disc_title(true)?;
        let track_count = self.get_track_count()? as u16;
        parse_track_group_list(&raw_title, &raw_full_title, track_count)
    }

    /// Writes the disc's group structure back to the device. Mirrors
    /// `rewriteDiscGroups` (`netmd-commands.ts:365`). Both raw titles are written
    /// through [`set_disc_title`], which sanitizes and length-prefixes them.
    pub fn rewrite_disc_groups(&self, disc: &Disc) -> Result<()> {
        debug!("rewrite disc groups");
        let compiled = compile_disc_titles(disc);
        self.set_disc_title(&compiled.raw_title, false)?;
        self.set_disc_title(&compiled.raw_full_width_title, true)?;
        Ok(())
    }
}

/// Pure core of [`get_track_group_list`], split out for testing.
fn parse_track_group_list(
    raw_title: &str,
    raw_full_title: &str,
    track_count: u16,
) -> Result<Vec<RawTrackGroup>> {
    let has_groups = raw_title.contains("//");
    let full_width_groups: Vec<&str> = raw_full_title.split("／／").collect();

    let mut result: Vec<RawTrackGroup> = Vec::new();
    let mut assigned: Vec<bool> = vec![false; track_count as usize];

    for group in raw_title.split("//") {
        if group.is_empty() {
            continue;
        }
        if group.starts_with("0;") || !group.contains(';') || !has_groups {
            continue;
        }

        let track_range = group.split(';').next().unwrap_or("");
        if track_range.is_empty() {
            continue;
        }
        let group_name = &group[track_range.len() + 1..];

        let full_width_range = half_width_to_full_width_range(track_range);
        let full_width_name = full_width_groups
            .iter()
            .find(|n| n.starts_with(&full_width_range))
            .map(|n| {
                n.chars()
                    .skip(full_width_range.chars().count() + 1)
                    .collect::<String>()
            });

        let (min_str, max_str) = match track_range.split_once('-') {
            Some((lo, hi)) => (lo, hi),
            None => (track_range, track_range),
        };
        let track_min: u16 = min_str.parse().map_err(|_| {
            NetMDError::UnexpectedResponse(format!("invalid group range {track_range:?}"))
        })?;
        let mut track_max: u16 = max_str.parse().map_err(|_| {
            NetMDError::UnexpectedResponse(format!("invalid group range {track_range:?}"))
        })?;
        track_max = track_max.min(track_count);
        if track_min < 1 || track_min > track_max {
            return Err(NetMDError::UnexpectedResponse(format!(
                "invalid group range {track_range:?}"
            )));
        }

        let mut track_list = Vec::new();
        for track in (track_min - 1)..track_max {
            let slot = assigned
                .get_mut(track as usize)
                .ok_or_else(|| NetMDError::UnexpectedResponse("group track out of range".into()))?;
            if *slot {
                return Err(NetMDError::UnexpectedResponse(format!(
                    "track {track} is in two groups"
                )));
            }
            *slot = true;
            track_list.push(track);
        }
        result.push(RawTrackGroup {
            name: Some(group_name.to_string()),
            full_width_name,
            tracks: track_list,
        });
    }

    let ungrouped: Vec<u16> = (0..track_count)
        .filter(|t| !assigned[*t as usize])
        .collect();
    if !ungrouped.is_empty() {
        result.insert(
            0,
            RawTrackGroup {
                name: None,
                full_width_name: None,
                tracks: ungrouped,
            },
        );
    }

    Ok(result)
}

#[must_use]
pub fn count_tracks_in_disc(disc: &Disc) -> usize {
    disc.groups.iter().map(|g| g.tracks.len()).sum()
}

#[must_use]
pub fn tracks(disc: &Disc) -> Vec<&Track> {
    disc.groups.iter().flat_map(|g| g.tracks.iter()).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TitleCells {
    pub half_width: usize,
    pub full_width: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemainingChars {
    pub half_width: usize,
    pub full_width: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledTitles {
    pub raw_title: String,
    pub raw_full_width_title: String,
}

#[must_use]
pub fn chars_to_cells(len: usize) -> usize {
    len.div_ceil(7)
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

#[must_use]
pub fn cells_for_title(track: &Track) -> TitleCells {
    let encoding_name_correction = if track.encoding == Encoding::Sp { 0 } else { 1 };
    let full_width_length =
        chars_to_cells(track.full_width_title.as_deref().map_or(0, utf16_len) * 2);
    let half_width_length = chars_to_cells(get_half_width_title_length(
        track.title.as_deref().unwrap_or(""),
    ));
    TitleCells {
        half_width: encoding_name_correction.max(half_width_length),
        full_width: encoding_name_correction.max(full_width_length),
    }
}

#[must_use]
pub fn remaining_characters_for_titles(disc: &Disc, include_groups: bool) -> RemainingChars {
    const CELL_LIMIT: usize = 255;

    let mut fw_title = format!("{}0;//", disc.full_width_title);
    let mut hw_title = format!("{}0;//", disc.title);

    if include_groups {
        for group in disc.groups.iter().filter(|g| g.title.is_some()) {
            let indices: Vec<u16> = group.tracks.iter().map(|t| t.index).collect();
            let (min, max) = match (indices.iter().min(), indices.iter().max()) {
                (Some(&lo), Some(&hi)) => (lo, hi),
                _ => continue,
            };
            let range = if group.tracks.len() - 1 != 0 {
                format!("{}-{}//", min + 1, max + 1)
            } else {
                format!("{}false//", min + 1)
            };
            fw_title.push_str(group.full_width_title.as_deref().unwrap_or(""));
            fw_title.push_str(&range);
            hw_title.push_str(group.title.as_deref().unwrap_or(""));
            hw_title.push_str(&range);
        }
    }

    let mut used_full = chars_to_cells(utf16_len(&fw_title) * 2);
    let mut used_half = chars_to_cells(get_half_width_title_length(&hw_title));
    for track in tracks(disc) {
        let cells = cells_for_title(track);
        used_half += cells.half_width;
        used_full += cells.full_width;
    }

    RemainingChars {
        half_width: CELL_LIMIT.saturating_sub(used_half) * 7,
        full_width: CELL_LIMIT.saturating_sub(used_full) * 7,
    }
}

#[must_use]
pub fn compile_disc_titles(disc: &Disc) -> CompiledTitles {
    let probe = Disc {
        title: String::new(),
        full_width_title: String::new(),
        ..disc.clone()
    };
    let RemainingChars {
        half_width: available_half,
        full_width: available_full,
    } = remaining_characters_for_titles(&probe, false);

    let use_full_width = !disc.full_width_title.is_empty()
        || disc.groups.iter().any(|g| {
            g.full_width_title.as_deref().is_some_and(|t| !t.is_empty())
                || g.tracks
                    .iter()
                    .any(|t| t.full_width_title.as_deref().is_some_and(|t| !t.is_empty()))
        });

    let mut new_raw_title = String::new();
    let mut new_raw_full_title = String::new();
    if !disc.title.is_empty() {
        new_raw_title = format!("0;{}//", disc.title);
    }
    if use_full_width {
        new_raw_full_title = format!("０；{}／／", disc.full_width_title);
    }

    let mut group_hits = 0;
    for group in &disc.groups {
        let Some(name) = group.title.as_deref() else {
            continue;
        };
        if group.tracks.is_empty() {
            continue;
        }
        group_hits += 1;
        let min_index = group.tracks.iter().map(|t| t.index).min().unwrap_or(0);
        let mut range = format!("{}", min_index + 1);
        if group.tracks.len() != 1 {
            range = format!("{range}-{}", min_index as usize + group.tracks.len());
        }

        let title_after = format!("{new_raw_title}{range};{name}//");
        let full_title_after = format!(
            "{new_raw_full_title}{}；{}／／",
            half_width_to_full_width_range(&range),
            group.full_width_title.as_deref().unwrap_or("")
        );

        let half_len_in_toc = chars_to_cells(get_half_width_title_length(&title_after)) * 7;
        if use_full_width {
            let full_len_in_toc = chars_to_cells(utf16_len(&full_title_after) * 2) * 7;
            if available_full >= full_len_in_toc {
                new_raw_full_title = full_title_after;
            }
        }
        if available_half >= half_len_in_toc {
            new_raw_title = title_after;
        }
    }

    if group_hits == 0 {
        new_raw_title = disc.title.clone();
        new_raw_full_title = disc.full_width_title.clone();
    }

    let half_len_in_toc = chars_to_cells(get_half_width_title_length(&new_raw_title)) * 7;
    let full_len_in_toc = chars_to_cells(utf16_len(&new_raw_full_title) * 2);
    if available_half < half_len_in_toc {
        new_raw_title = String::new();
    }
    if available_full < full_len_in_toc {
        new_raw_full_title = String::new();
    }

    CompiledTitles {
        raw_title: new_raw_title,
        raw_full_width_title: if use_full_width {
            new_raw_full_title
        } else {
            String::new()
        },
    }
}

impl Disc {
    fn named_ranges(&self) -> Vec<(u16, u16, String, Option<String>)> {
        let mut ranges: Vec<(u16, u16, String, Option<String>)> = self
            .groups
            .iter()
            .filter_map(|g| {
                let name = g.title.clone()?;
                let first = g.tracks.iter().map(|t| t.index).min()?;
                let last = g.tracks.iter().map(|t| t.index).max()?;
                Some((first, last, name, g.full_width_title.clone()))
            })
            .collect();
        ranges.sort_by_key(|r| r.0);
        ranges
    }

    fn flat_tracks(&self) -> Vec<Track> {
        let mut all: Vec<Track> = self
            .groups
            .iter()
            .flat_map(|g| g.tracks.iter().cloned())
            .collect();
        all.sort_by_key(|t| t.index);
        all
    }

    fn rebuild(&mut self, mut named: Vec<(u16, u16, String, Option<String>)>) {
        named.sort_by_key(|r| r.0);
        let flat = self.flat_tracks();
        let assigned: std::collections::HashSet<u16> = named
            .iter()
            .flat_map(|(first, last, _, _)| *first..=*last)
            .collect();

        let mut groups: Vec<Group> = Vec::new();
        let mut next_index = 0usize;

        let ungrouped: Vec<Track> = flat
            .iter()
            .filter(|t| !assigned.contains(&t.index))
            .cloned()
            .collect();
        if !ungrouped.is_empty() {
            groups.push(Group {
                index: next_index,
                title: None,
                full_width_title: None,
                tracks: ungrouped,
            });
            next_index += 1;
        }

        for (first, last, name, full) in named {
            let group_tracks: Vec<Track> = flat
                .iter()
                .filter(|t| t.index >= first && t.index <= last)
                .cloned()
                .collect();
            groups.push(Group {
                index: next_index,
                title: Some(name),
                full_width_title: full,
                tracks: group_tracks,
            });
            next_index += 1;
        }

        self.groups = groups;
    }

    /// Creates a new group spanning the contiguous, 0-based track range
    /// `first..=last`. The tracks must currently be ungrouped.
    pub fn add_group(
        &mut self,
        first: u16,
        last: u16,
        name: String,
        full_name: Option<String>,
    ) -> Result<()> {
        if first > last {
            return Err(NetMDError::UnexpectedResponse(
                "group start must not exceed end".into(),
            ));
        }
        if (last as usize) >= self.track_count as usize {
            return Err(NetMDError::UnexpectedResponse(format!(
                "track {} out of range (disc has {} tracks)",
                last + 1,
                self.track_count
            )));
        }
        let mut ranges = self.named_ranges();
        for (rf, rl, _, _) in &ranges {
            if first <= *rl && *rf <= last {
                return Err(NetMDError::UnexpectedResponse(
                    "range overlaps an existing group; remove it first".into(),
                ));
            }
        }
        ranges.push((first, last, name, full_name));
        self.rebuild(ranges);
        Ok(())
    }

    /// Renames the named group at the given group index (as reported by the
    /// listing). `full_name` of `None` leaves the full-width title untouched.
    pub fn rename_group(
        &mut self,
        group_index: usize,
        name: String,
        full_name: Option<String>,
    ) -> Result<()> {
        let group = self
            .groups
            .iter_mut()
            .find(|g| g.index == group_index && g.title.is_some())
            .ok_or_else(|| {
                NetMDError::UnexpectedResponse(format!("no group at index {group_index}"))
            })?;
        group.title = Some(name);
        if full_name.is_some() {
            group.full_width_title = full_name;
        }
        Ok(())
    }

    /// Dissolves the named group at the given group index, returning its tracks
    /// to the ungrouped bucket.
    pub fn remove_group(&mut self, group_index: usize) -> Result<()> {
        let target = self
            .groups
            .iter()
            .find(|g| g.index == group_index && g.title.is_some())
            .map(|g| g.tracks.iter().map(|t| t.index).min())
            .ok_or_else(|| {
                NetMDError::UnexpectedResponse(format!("no group at index {group_index}"))
            })?;
        let ranges: Vec<_> = self
            .named_ranges()
            .into_iter()
            .filter(|(first, _, _, _)| Some(*first) != target)
            .collect();
        self.rebuild(ranges);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChannelCount, TrackFlag};

    fn raw_group(name: Option<&str>, tracks: &[u16]) -> RawTrackGroup {
        RawTrackGroup {
            name: name.map(str::to_string),
            full_width_name: None,
            tracks: tracks.to_vec(),
        }
    }

    #[test]
    fn parse_groups_with_disc_title_and_ungrouped() {
        let got = parse_track_group_list("0;Disc//1-3;GroupA//5;GroupB//", "", 6).unwrap();
        assert_eq!(
            got,
            vec![
                raw_group(None, &[3, 5]),
                raw_group(Some("GroupA"), &[0, 1, 2]),
                raw_group(Some("GroupB"), &[4]),
            ]
        );
    }

    #[test]
    fn parse_groups_no_groups_returns_single_ungrouped() {
        let got = parse_track_group_list("Just a title", "", 3).unwrap();
        assert_eq!(got, vec![raw_group(None, &[0, 1, 2])]);
    }

    #[test]
    fn parse_groups_full_width_name_matched_by_range() {
        let got = parse_track_group_list(
            "0;Disc//1-2;Hello//",
            "０；Ｄｉｓｃ／／１－２；ハロー／／",
            2,
        )
        .unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name.as_deref(), Some("Hello"));
        assert_eq!(got[0].full_width_name.as_deref(), Some("ハロー"));
    }

    #[test]
    fn parse_groups_detects_overlap() {
        let err = parse_track_group_list("0;D//1-3;A//2-4;B//", "", 4);
        assert!(err.is_err());
    }

    #[test]
    fn chars_to_cells_rounds_up() {
        assert_eq!(chars_to_cells(0), 0);
        assert_eq!(chars_to_cells(1), 1);
        assert_eq!(chars_to_cells(7), 1);
        assert_eq!(chars_to_cells(8), 2);
    }

    fn track(index: u16, title: &str, encoding: Encoding) -> Track {
        Track {
            index,
            title: if title.is_empty() {
                None
            } else {
                Some(title.to_string())
            },
            full_width_title: None,
            duration_frames: 0,
            channel: ChannelCount::Stereo,
            encoding,
            protected: TrackFlag::Unprotected,
        }
    }

    fn disc_with(title: &str, groups: Vec<Group>, track_count: u8) -> Disc {
        Disc {
            title: title.to_string(),
            full_width_title: String::new(),
            writable: true,
            write_protected: false,
            used: 0,
            left: 0,
            total: 0,
            track_count,
            groups,
        }
    }

    #[test]
    fn cells_for_title_reserves_a_cell_for_lp_tracks() {
        assert_eq!(cells_for_title(&track(0, "", Encoding::Sp)).half_width, 0);
        assert_eq!(cells_for_title(&track(0, "", Encoding::Lp2)).half_width, 1);
        assert_eq!(
            cells_for_title(&track(0, "12345678", Encoding::Sp)).half_width,
            2
        );
    }

    #[test]
    fn compile_titles_packs_groups() {
        let groups = vec![
            Group {
                index: 0,
                title: Some("First".into()),
                full_width_title: None,
                tracks: vec![track(0, "a", Encoding::Sp), track(1, "b", Encoding::Sp)],
            },
            Group {
                index: 1,
                title: Some("Second".into()),
                full_width_title: None,
                tracks: vec![track(2, "c", Encoding::Sp)],
            },
        ];
        let disc = disc_with("MyDisc", groups, 3);
        let compiled = compile_disc_titles(&disc);
        assert_eq!(compiled.raw_title, "0;MyDisc//1-2;First//3;Second//");
        assert_eq!(compiled.raw_full_width_title, "");
    }

    #[test]
    fn compile_titles_no_groups_uses_plain_title() {
        let disc = disc_with(
            "Plain",
            vec![Group {
                index: 0,
                title: None,
                full_width_title: None,
                tracks: vec![track(0, "a", Encoding::Sp)],
            }],
            1,
        );
        let compiled = compile_disc_titles(&disc);
        assert_eq!(compiled.raw_title, "Plain");
    }

    #[test]
    fn add_and_remove_group_round_trip() {
        let mut disc = disc_with(
            "D",
            vec![Group {
                index: 0,
                title: None,
                full_width_title: None,
                tracks: vec![
                    track(0, "a", Encoding::Sp),
                    track(1, "b", Encoding::Sp),
                    track(2, "c", Encoding::Sp),
                ],
            }],
            3,
        );
        disc.add_group(0, 1, "Grp".into(), None).unwrap();
        assert_eq!(disc.groups.len(), 2);
        assert_eq!(disc.groups[0].title, None);
        assert_eq!(
            disc.groups[0]
                .tracks
                .iter()
                .map(|t| t.index)
                .collect::<Vec<_>>(),
            vec![2]
        );
        assert_eq!(disc.groups[1].title.as_deref(), Some("Grp"));
        assert_eq!(
            disc.groups[1]
                .tracks
                .iter()
                .map(|t| t.index)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );

        assert!(disc.add_group(1, 2, "X".into(), None).is_err());

        let grp_index = disc.groups[1].index;
        disc.remove_group(grp_index).unwrap();
        assert_eq!(disc.groups.len(), 1);
        assert_eq!(disc.groups[0].title, None);
        assert_eq!(disc.groups[0].tracks.len(), 3);
    }
}
