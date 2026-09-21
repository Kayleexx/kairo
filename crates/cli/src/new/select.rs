use std::path::PathBuf;

use kairo_core::{ComponentHash, Config};
use kairo_runtime::{CatalogComponent, ComponentRole};

use super::{NewError, default_step_name, render};

#[derive(Clone)]
pub(super) struct Selected {
    pub(super) path: PathBuf,
    pub(super) hash: Option<ComponentHash>,
    pub(super) name: String,
    pub(super) role: ComponentRole,
}

// same producer/consumer wording as the recipe path, instead of a raw WIT export-mismatch error.
pub(super) fn check_connections(components: &[PathBuf], config: Config) -> Result<(), NewError> {
    let mut previous = None;
    for component in components {
        let contract = render::detect_contract(component, config)?;
        if !contract.role.can_follow(previous) {
            return Err(NewError::ComponentConnection {
                previous: previous
                    .map_or("workflow input", ComponentRole::shape)
                    .to_owned(),
                next: contract.role.shape().to_owned(),
            });
        }
        previous = Some(contract.role);
    }
    Ok(())
}

// picks by number or catalog name, never a raw path; order is inferred when unambiguous.
pub(super) fn select_from_catalog(
    catalog: &[CatalogComponent],
    config: Config,
) -> Result<Vec<Selected>, NewError> {
    loop {
        print_catalog(catalog);
        let input = crate::prompt::ask(
            "select components by number or name (space-separated), or a path to import",
            "",
        )?;
        if input.is_empty() {
            println!("error: select at least one component");
            continue;
        }
        let mut selected = Vec::new();
        let mut failed = false;
        for token in input.split_whitespace() {
            match resolve_token(token, catalog, config, &selected) {
                Ok(item) => selected.push(item),
                Err(message) => {
                    println!("error: {message}");
                    failed = true;
                    break;
                }
            }
        }
        if failed {
            continue;
        }
        if let Some(chosen) = resolve_chain(selected)? {
            return Ok(chosen);
        }
    }
}

// nothing registered yet -- two explicit choices instead of one prompt that guesses intent.
pub(super) fn select_by_import(config: Config) -> Result<Vec<Selected>, NewError> {
    println!("no Components are registered yet\n");
    loop {
        println!("  1. add an existing Component (a local file path)");
        println!("  2. scaffold a new Component\n");
        let choice = crate::prompt::ask("choice", "1")?;
        match choice.trim() {
            "1" => {
                if let Some(selected) = add_existing(config)? {
                    return Ok(selected);
                }
            }
            "2" => {
                if let Some(name) = scaffold_new()? {
                    return Err(name);
                }
            }
            _ => println!("error: enter 1 or 2\n"),
        }
    }
}

fn add_existing(config: Config) -> Result<Option<Vec<Selected>>, NewError> {
    let raw = crate::prompt::required("component path")?;
    let path = PathBuf::from(&raw);
    if !path.is_file() {
        println!(
            "error: `{raw}` was not found\n  for an OCI Component, run `kairo add {raw}` first, \
             then `kairo new` again\n"
        );
        return Ok(None);
    }
    match render::detect_contract(&path, config) {
        Ok(contract) => {
            let name = default_step_name(&path, 0);
            Ok(Some(vec![Selected {
                path,
                hash: Some(contract.hash),
                name,
                role: contract.role,
            }]))
        }
        Err(error) => {
            println!("error: {error}\n");
            Ok(None)
        }
    }
}

// `Ok(Some(_))` exits with a `Cancelled` message -- a fresh scaffold isn't usable yet.
fn scaffold_new() -> Result<Option<NewError>, NewError> {
    let raw = crate::prompt::required("component name")?;
    if !crate::component::valid_name(&raw) {
        println!(
            "error: use 1-64 lowercase letters, digits, `-`, or `_`, starting with a letter\n"
        );
        return Ok(None);
    }
    let directory = crate::component::new(&raw)?;
    crate::print_valid(format!(
        "component · {}\nnext:\n  1. implement its src/lib.rs\n  2. kairo component build {}\n  3. kairo add {}/component.wasm\n  4. kairo new (retry)",
        directory.display(),
        directory.display(),
        directory.display()
    ));
    Ok(Some(NewError::Cancelled))
}

pub(super) fn ask_yes_no(prompt: &str, default: bool) -> Result<bool, NewError> {
    loop {
        let value = crate::prompt::ask(prompt, if default { "yes" } else { "no" })?;
        match value.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => println!("error: answer \"yes\" or \"no\""),
        }
    }
}

fn print_catalog(catalog: &[CatalogComponent]) {
    println!("Select components:\n");
    for (index, component) in catalog.iter().enumerate() {
        println!("  {}. {}", index + 1, component.entry.name);
        println!("     {}", component.contract.shape());
    }
    println!();
}

fn resolve_token(
    token: &str,
    catalog: &[CatalogComponent],
    config: Config,
    selected: &[Selected],
) -> Result<Selected, String> {
    if let Ok(number) = token.parse::<usize>() {
        let component = number
            .checked_sub(1)
            .and_then(|index| catalog.get(index))
            .ok_or_else(|| format!("no component numbered {token}"))?;
        return Ok(from_catalog(component, selected));
    }
    if let Some(component) = catalog
        .iter()
        .find(|component| component.entry.name == token)
    {
        return Ok(from_catalog(component, selected));
    }
    let path = PathBuf::from(token);
    if path.is_file() {
        let contract = render::detect_contract(&path, config).map_err(|error| error.to_string())?;
        let name = unique_step_name(selected, &default_step_name(&path, selected.len()));
        return Ok(Selected {
            path,
            hash: Some(contract.hash),
            name,
            role: contract.role,
        });
    }
    Err(format!(
        "`{token}` is not a registered Component, a valid number, or an existing file"
    ))
}

fn from_catalog(component: &CatalogComponent, selected: &[Selected]) -> Selected {
    Selected {
        path: component.entry.path.clone(),
        hash: Some(component.contract.hash),
        name: unique_step_name(selected, &component.entry.name),
        role: component.contract.role,
    }
}

fn unique_step_name(selected: &[Selected], base: &str) -> String {
    if !selected.iter().any(|item| item.name == base) {
        return base.to_owned();
    }
    let mut suffix = 2;
    loop {
        let candidate = format!("{base}-{suffix}");
        if !selected.iter().any(|item| item.name == candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

// `None` means reselect (explained mismatch or a declined ambiguous choice), not failure.
fn resolve_chain(selected: Vec<Selected>) -> Result<Option<Vec<Selected>>, NewError> {
    if is_valid_order(&selected) {
        return Ok(Some(selected));
    }
    let mut orderings = valid_orderings(&selected);
    match orderings.len() {
        0 => {
            explain_incompatible(&selected);
            Ok(None)
        }
        1 => Ok(Some(orderings.remove(0))),
        _ => choose_ordering(orderings),
    }
}

fn is_valid_order(selected: &[Selected]) -> bool {
    let mut previous = None;
    for item in selected {
        if !item.role.can_follow(previous) {
            return false;
        }
        previous = Some(item.role);
    }
    true
}

fn valid_orderings(selected: &[Selected]) -> Vec<Vec<Selected>> {
    let mut out = Vec::new();
    let indices: Vec<usize> = (0..selected.len()).collect();
    permute(&indices, &mut Vec::new(), selected, &mut out);
    out
}

fn permute(
    remaining: &[usize],
    prefix: &mut Vec<usize>,
    selected: &[Selected],
    out: &mut Vec<Vec<Selected>>,
) {
    if remaining.is_empty() {
        out.push(
            prefix
                .iter()
                .map(|&index| selected[index].clone())
                .collect(),
        );
        return;
    }
    let previous = prefix.last().map(|&index| selected[index].role);
    for (position, &index) in remaining.iter().enumerate() {
        if selected[index].role.can_follow(previous) {
            let mut next_remaining = remaining.to_vec();
            next_remaining.remove(position);
            prefix.push(index);
            permute(&next_remaining, prefix, selected, out);
            prefix.pop();
        }
    }
}

fn choose_ordering(orderings: Vec<Vec<Selected>>) -> Result<Option<Vec<Selected>>, NewError> {
    println!("multiple valid connection orders exist:\n");
    for (index, ordering) in orderings.iter().enumerate() {
        println!("  {}. {}", index + 1, chain_label(ordering));
    }
    let choice = crate::prompt::ask("\nchoose an order (blank to reselect)", "")?;
    if choice.is_empty() {
        return Ok(None);
    }
    let Some(chosen) = choice
        .parse::<usize>()
        .ok()
        .and_then(|number| number.checked_sub(1))
        .and_then(|index| orderings.get(index))
    else {
        println!("error: invalid choice");
        return Ok(None);
    };
    Ok(Some(chosen.clone()))
}

fn explain_incompatible(selected: &[Selected]) {
    println!("error: these Components cannot form a valid chain:");
    for item in selected {
        println!("  {} · {}", item.name, item.role.shape());
    }
    println!("try a different selection");
}

pub(super) fn chain_label(selected: &[Selected]) -> String {
    selected
        .iter()
        .map(|item| item.name.as_str())
        .collect::<Vec<_>>()
        .join(" -> ")
}
