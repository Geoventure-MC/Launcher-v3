//! Réglages légers du jeu pour le profil Économe (`options.txt`).
//!
//! Règle de l'ancien launcher : on ne touche jamais aux réglages du joueur sans son accord.
//! Ici, seulement sur demande, on ne fait que BAISSER des valeurs, et on garde une copie
//! (`options.txt.avant-econome`) qui permet de tout remettre comme avant.

use std::path::Path;

pub const BACKUP_NAME: &str = "options.txt.avant-econome";

/// Clé, valeur légère, et comment comparer : `Max(n)` = plafond numérique, `Set` = valeur imposée.
enum Rule {
    Max(i64),
    Set(&'static str),
}

const LIGHT: &[(&str, Rule)] = &[
    ("renderDistance", Rule::Max(8)),
    ("simulationDistance", Rule::Max(6)),
    ("graphicsMode", Rule::Max(0)),
    ("biomeBlendRadius", Rule::Max(1)),
    ("mipmapLevels", Rule::Max(2)),
    ("entityDistanceScaling", Rule::Max(1)),
    ("particles", Rule::Set("2")),
    ("entityShadows", Rule::Set("false")),
    ("renderClouds", Rule::Set("\"false\"")),
];

/// Applique les réglages légers à un texte `options.txt`. Renvoie le texte et les clés changées.
pub fn lighten(text: &str) -> (String, Vec<String>) {
    let mut changed = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<String> = text
        .lines()
        .map(|line| {
            let Some((k, v)) = line.split_once(':') else { return line.to_string() };
            let Some((_, rule)) = LIGHT.iter().find(|(key, _)| *key == k) else { return line.to_string() };
            seen.insert(k.to_string());
            let new_v = match rule {
                Rule::Max(max) => match v.trim().parse::<f64>() {
                    Ok(n) if n > *max as f64 => max.to_string(),
                    Ok(_) => return line.to_string(),
                    Err(_) => max.to_string(),
                },
                Rule::Set(val) => {
                    if v.trim() == *val {
                        return line.to_string();
                    }
                    val.to_string()
                }
            };
            changed.push(k.to_string());
            format!("{k}:{new_v}")
        })
        .collect();
    for (k, rule) in LIGHT {
        if !seen.contains(*k) {
            let v = match rule {
                Rule::Max(m) => m.to_string(),
                Rule::Set(s) => s.to_string(),
            };
            out.push(format!("{k}:{v}"));
            changed.push(k.to_string());
        }
    }
    let mut s = out.join("\n");
    s.push('\n');
    (s, changed)
}

/// Baisse les réglages du jeu dans `<game_dir>/options.txt` (copie de sauvegarde la première fois).
pub fn apply_light(game_dir: &Path) -> std::io::Result<Vec<String>> {
    std::fs::create_dir_all(game_dir)?;
    let path = game_dir.join("options.txt");
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    let backup = game_dir.join(BACKUP_NAME);
    if !backup.exists() {
        std::fs::write(&backup, &current)?;
    }
    let (text, changed) = lighten(&current);
    if !changed.is_empty() {
        std::fs::write(&path, text)?;
    }
    Ok(changed)
}

/// Remet `options.txt` comme avant le premier passage en Économe. `false` s'il n'y a pas de copie.
pub fn restore(game_dir: &Path) -> std::io::Result<bool> {
    let backup = game_dir.join(BACKUP_NAME);
    if !backup.exists() {
        return Ok(false);
    }
    let saved = std::fs::read_to_string(&backup)?;
    let path = game_dir.join("options.txt");
    if saved.is_empty() {
        let _ = std::fs::remove_file(&path);
    } else {
        std::fs::write(&path, saved)?;
    }
    std::fs::remove_file(&backup)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_lowers_values() {
        let (s, changed) = lighten("renderDistance:16\nsimulationDistance:4\nlang:fr_fr\nparticles:0\n");
        assert!(s.contains("renderDistance:8"));
        assert!(s.contains("simulationDistance:4"), "déjà bas : inchangé");
        assert!(s.contains("lang:fr_fr"));
        assert!(s.contains("particles:2"));
        assert!(changed.contains(&"renderDistance".to_string()));
        assert!(!changed.contains(&"simulationDistance".to_string()));
        assert!(s.contains("entityShadows:false"), "clé absente ajoutée");
    }

    #[test]
    fn applies_then_restores() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("options.txt"), "renderDistance:20\nlang:fr_fr\n").unwrap();
        let changed = apply_light(tmp.path()).unwrap();
        assert!(!changed.is_empty());
        // Deuxième passage : la copie garde l'original.
        apply_light(tmp.path()).unwrap();
        assert!(restore(tmp.path()).unwrap());
        let s = std::fs::read_to_string(tmp.path().join("options.txt")).unwrap();
        assert_eq!(s, "renderDistance:20\nlang:fr_fr\n");
        assert!(!restore(tmp.path()).unwrap());
    }
}
