//! Sync git API for fetching a file blame

use super::{utils, CommitId};
use crate::{
	error::{Error, Result},
	sync::get_commits_info,
};
use scopetime::scope_time;
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::Path;

/// A `BlameHunk` contains all the information that will be shown to the user.
#[derive(Clone, Hash, Debug, PartialEq, Eq)]
pub struct BlameHunk {
	///
	pub commit_id: CommitId,
	///
	pub author: String,
	///
	pub time: i64,
	/// `git2::BlameHunk::final_start_line` returns 1-based indices, but
	/// `start_line` is 0-based because the `Vec` storing the lines starts at
	/// index 0.
	pub start_line: usize,
	///
	pub end_line: usize,
}

/// A `BlameFile` represents a collection of lines. This is targeted at how the
/// data will be used by the UI.
#[derive(Clone, Debug)]
pub struct FileBlame {
	///
	pub commit_id: CommitId,
	///
	pub path: String,
	///
	pub lines: Vec<(Option<BlameHunk>, String)>,
}

///
pub fn blame_file(
	repo_path: &str,
	file_path: &str,
) -> Result<FileBlame> {
	scope_time!("blame_file");

	let repo = utils::repo(repo_path)?;

	let commit_id = utils::get_head_repo(&repo)?;

	let spec = format!("{}:{}", commit_id.to_string(), file_path);

	let object = repo.revparse_single(&spec)?;
	let blob = repo.find_blob(object.id())?;

	if blob.is_binary() {
		return Err(Error::NoBlameOnBinaryFile);
	}

	let blame = repo.blame_file(Path::new(file_path), None)?;

	let reader = BufReader::new(blob.content());

	let unique_commit_ids: HashSet<_> = blame
		.iter()
		.map(|hunk| CommitId::new(hunk.final_commit_id()))
		.collect();
	let mut commit_ids = Vec::with_capacity(unique_commit_ids.len());
	commit_ids.extend(unique_commit_ids);

	let commit_infos = get_commits_info(repo_path, &commit_ids, 0)?;
	let unique_commit_infos: HashMap<_, _> = commit_infos
		.iter()
		.map(|commit_info| (commit_info.id, commit_info))
		.collect();

	let lines: Vec<(Option<BlameHunk>, String)> = reader
		.lines()
		.enumerate()
		.map(|(i, line)| {
			// Line indices in a `FileBlame` are 1-based.
			let corresponding_hunk = blame.get_line(i + 1);

			if let Some(hunk) = corresponding_hunk {
				let commit_id = CommitId::new(hunk.final_commit_id());
				// Line indices in a `BlameHunk` are 1-based.
				let start_line =
					hunk.final_start_line().saturating_sub(1);
				let end_line =
					start_line.saturating_add(hunk.lines_in_hunk());

				if let Some(commit_info) =
					unique_commit_infos.get(&commit_id)
				{
					let hunk = BlameHunk {
						commit_id,
						author: commit_info.author.clone(),
						time: commit_info.time,
						start_line,
						end_line,
					};

					return (
						Some(hunk),
						line.unwrap_or_else(|_| "".into()),
					);
				}
			}

			(None, line.unwrap_or_else(|_| "".into()))
		})
		.collect();

	let file_blame = FileBlame {
		commit_id,
		path: file_path.into(),
		lines,
	};

	Ok(file_blame)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::error::Result;
	use crate::sync::{
		commit, stage_add_file, tests::repo_init_empty,
	};
	use std::{
		fs::{File, OpenOptions},
		io::Write,
		path::Path,
	};

	#[test]
	fn test_blame() -> Result<()> {
		let file_path = Path::new("foo");
		let (_td, repo) = repo_init_empty()?;
		let root = repo.path().parent().unwrap();
		let repo_path = root.as_os_str().to_str().unwrap();

		assert!(matches!(blame_file(&repo_path, "foo"), Err(_)));

		File::create(&root.join(file_path))?
			.write_all(b"line 1\n")?;

		stage_add_file(repo_path, file_path)?;
		commit(repo_path, "first commit")?;

		let blame = blame_file(&repo_path, "foo")?;

		assert!(matches!(
			blame.lines.as_slice(),
			[(
				Some(BlameHunk {
					author,
					start_line: 0,
					end_line: 1,
					..
				}),
				line
			)] if author == "name" && line == "line 1"
		));

		let mut file = OpenOptions::new()
			.append(true)
			.open(&root.join(file_path))?;

		file.write(b"line 2\n")?;

		stage_add_file(repo_path, file_path)?;
		commit(repo_path, "second commit")?;

		let blame = blame_file(&repo_path, "foo")?;

		assert!(matches!(
			blame.lines.as_slice(),
			[
				(
					Some(BlameHunk {
						start_line: 0,
						end_line: 1,
						..
					}),
					first_line
				),
				(
					Some(BlameHunk {
						author,
						start_line: 1,
						end_line: 2,
						..
					}),
					second_line
				)
			] if author == "name" && first_line == "line 1" && second_line == "line 2"
		));

		file.write(b"line 3\n")?;

		let blame = blame_file(&repo_path, "foo")?;

		assert_eq!(blame.lines.len(), 2);

		stage_add_file(repo_path, file_path)?;
		commit(repo_path, "third commit")?;

		let blame = blame_file(&repo_path, "foo")?;

		assert_eq!(blame.lines.len(), 3);

		Ok(())
	}
}
