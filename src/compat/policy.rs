use anyhow::{Result, ensure};
pub const DEFAULT_GROUP: &str = "user_primary";
pub fn project(group: &str) -> bool {
    let Some(value) = group.strip_prefix("project_") else {
        return false;
    };
    let Some((name, hash)) = value.rsplit_once('_') else {
        return false;
    };
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        && hash.len() == 8
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn read(group: &str) -> Result<()> {
    ensure!(group.len() <= 128, "group exceeds length bound");
    ensure!(
        matches!(
            group,
            "global_user" | "user_primary" | "general_chat" | "hermes" | "family_shared"
        ) || project(group),
        "group is not readable"
    );
    Ok(())
}
pub fn write(group: &str) -> Result<()> {
    read(group)?;
    ensure!(
        group != "hermes" && group != "family_shared",
        "group is read-only"
    );
    Ok(())
}
pub fn groups(groups: Option<&[String]>) -> Result<Vec<String>> {
    let default = vec![DEFAULT_GROUP.to_string()];
    let supplied = groups.unwrap_or(&default);
    ensure!(
        !supplied.is_empty() && supplied.len() <= 16,
        "group_ids must contain 1..=16 groups"
    );
    let mut result = vec![];
    for group in supplied {
        read(group)?;
        if !result.contains(group) {
            result.push(group.clone());
        }
    }
    Ok(result)
}
pub fn clear(groups: Option<&[String]>) -> Result<&str> {
    let groups =
        groups.ok_or_else(|| anyhow::anyhow!("clear_graph requires one explicit group"))?;
    ensure!(
        groups.len() == 1,
        "clear_graph requires exactly one explicit group"
    );
    write(&groups[0])?;
    Ok(&groups[0])
}
pub fn uuid(value: &str) -> Result<()> {
    ensure!(uuid::Uuid::parse_str(value).is_ok(), "invalid UUID");
    Ok(())
}
pub fn limit(value: i64) -> Result<usize> {
    ensure!((1..=100).contains(&value), "result limit must be 1..=100");
    Ok(value as usize)
}
