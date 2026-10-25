use axum::{
    extract::{Path, State},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use crate::state::SharedState;
use crate::store::db::StoredSource;
use crate::error::Result;
use chrono::Utc;

#[derive(Deserialize)]
pub struct CreateSourceRequest {
    #[serde(default)]
    pub name: Option<String>,
    pub source_type: String, // "clash_api", "clash_dir", "clash_file"
    pub target: String,
    pub secret: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Automatically infers a clean, human-readable source name when the user doesn't provide one
pub fn infer_source_name(target: &str, source_type: &str) -> String {
    match source_type {
        "clash_api" => {
            let clean = target
                .trim()
                .trim_start_matches("http://")
                .trim_start_matches("https://")
                .trim_end_matches('/');
            format!("Clash API ({})", clean)
        }
        "clash_file" => {
            let path = std::path::Path::new(target);
            let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            let file_stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");

            // Check metadata if this file is in a known or parent directory
            let parent_dirs = path.parent().map(|p| vec![p.to_path_buf()]).unwrap_or_default();
            let meta = crate::collector::detector::ClashDetector::load_profiles_metadata_with_custom(&parent_dirs);

            if let Some((name, _)) = meta.get(file_name).or_else(|| meta.get(file_stem)) {
                if !name.trim().is_empty() {
                    return name.trim().to_string();
                }
            }
            if !file_stem.is_empty() {
                file_stem.to_string()
            } else if !file_name.is_empty() {
                file_name.to_string()
            } else {
                "本地订阅".to_string()
            }
        }
        "clash_dir" => {
            let path = std::path::Path::new(target);
            path.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("本地配置目录")
                .to_string()
        }
        _ => "未知源".to_string(),
    }
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
pub struct ToggleSourceRequest {
    pub enabled: bool,
}

#[derive(Deserialize)]
pub struct UpdateSourceRequest {
    pub name: Option<String>,
    pub target: Option<String>,
    pub secret: Option<String>,
    pub enabled: Option<bool>,
}

pub async fn list_sources(
    State(state): State<SharedState>,
) -> Result<Json<Value>> {
    let sources = state.db.get_all_sources().await?;
    Ok(Json(json!({
        "success": true,
        "count": sources.len(),
        "data": sources
    })))
}

/// Explicit on-demand scan: scans local Clash profiles and imports new ones only when user requests
pub async fn scan_sources(
    State(state): State<SharedState>,
) -> Result<Json<Value>> {
    let (discovered_count, newly_added) = state.refresh_discovered_sources().await?;
    let count = state.sync_nodes_and_users(false).await?;
    let _ = state.apply_and_reload().await;
    *state.status_cache.lock().await = None;

    let sources = state.db.get_all_sources().await?;

    let message = if newly_added > 0 {
        format!(
            "扫描完成：新发现并自动导入 {} 个本地订阅配置，当前共汇聚 {} 个有效节点",
            newly_added, count
        )
    } else {
        format!(
            "扫描完成：已扫描本机 Clash 目录，共探测到 {} 个有效配置文件，全部已在管理列表中（无新增），已同步刷新节点数据（当前共 {} 节点）",
            discovered_count, count
        )
    };

    Ok(Json(json!({
        "success": true,
        "message": message,
        "discovered_count": discovered_count,
        "newly_added": newly_added,
        "count": sources.len(),
        "data": sources,
        "total_nodes": count
    })))
}

pub async fn create_source(
    State(state): State<SharedState>,
    Json(payload): Json<CreateSourceRequest>,
) -> Result<Json<Value>> {
    let target = payload.target.trim().to_string();
    if target.is_empty() {
        return Err(crate::error::AppError::Config("目标路径或接口地址不能为空".to_string()));
    }

    // Auto-infer or generate friendly name if not explicitly provided
    let name = payload.name
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| infer_source_name(&target, &payload.source_type));

    let secret = payload.secret.and_then(|s| {
        let trimmed = s.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });

    let existing = state.db.get_all_sources().await?;

    // Check duplicate targets: never add the same file or API twice
    if payload.source_type == "clash_file" {
        let p = std::path::Path::new(&target);
        let _target_canonical = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());

        for s in &existing {
            if s.source_type == "clash_file" {
                if s.target == target {
                    return Err(crate::error::AppError::Config(format!(
                        "该文件路径已登记为配置源「{}」，不能重复添加", s.name
                    )));
                }
                let s_p = std::path::Path::new(&s.target);
                if let (Ok(c1), Ok(c2)) = (std::fs::canonicalize(p), std::fs::canonicalize(s_p)) {
                    if c1 == c2 {
                        return Err(crate::error::AppError::Config(format!(
                            "该文件路径已登记为配置源「{}」，不能重复添加", s.name
                        )));
                    }
                }
            }
        }
    } else if payload.source_type == "clash_api" {
        let norm_target = target.trim().trim_end_matches('/').to_lowercase();
        for s in &existing {
            if s.source_type == "clash_api" {
                let norm_s = s.target.trim().trim_end_matches('/').to_lowercase();
                if norm_s == norm_target {
                    return Err(crate::error::AppError::Config(format!(
                        "该接口地址已登记为配置源「{}」，不能重复添加", s.name
                    )));
                }
            }
        }
    }

    // If source type is a local directory: scan it and register EACH valid configuration file as its own separate source!
    // Never lump files together into a single opaque row.
    if payload.source_type == "clash_dir" {
        let dir = std::path::Path::new(&target);
        if !dir.exists() || !dir.is_dir() {
            return Err(crate::error::AppError::Config(format!("指定的目录不存在或不是文件夹: {}", target)));
        }

        let custom_dirs = vec![dir.to_path_buf()];
        let profiles = crate::collector::detector::ClashDetector::discover_profile_sources_with_custom(&custom_dirs);

        let existing_targets: std::collections::HashSet<String> = existing.iter().map(|s| s.target.clone()).collect();
        let existing_ids: std::collections::HashSet<String> = existing.iter().map(|s| s.id.clone()).collect();
        let existing_canonicals: std::collections::HashSet<std::path::PathBuf> = existing
            .iter()
            .map(|s| std::fs::canonicalize(&s.target).unwrap_or_else(|_| std::path::PathBuf::from(&s.target)))
            .collect();

        let mut added_sources = Vec::new();

        for p in profiles {
            let target_str = p.path.to_string_lossy().to_string();
            let canonical_p = std::fs::canonicalize(&p.path).unwrap_or_else(|_| p.path.clone());

            // Check if this file belongs to the scanned directory
            let in_dir = p.path.starts_with(dir) 
                || canonical_p.starts_with(dir)
                || dir.canonicalize().map(|cd| canonical_p.starts_with(&cd)).unwrap_or(false);

            if in_dir {
                // Deduplicate: check id, raw target, and canonical path
                let is_dup = existing_ids.contains(&p.id) 
                    || existing_targets.contains(&target_str)
                    || existing_canonicals.contains(&canonical_p);

                if !is_dup {
                    let s = StoredSource {
                        id: p.id.clone(),
                        name: p.name,
                        source_type: "clash_file".to_string(),
                        target: target_str,
                        secret: None,
                        enabled: payload.enabled,
                        created_at: Utc::now(),
                    };
                    state.db.save_source(&s).await?;
                    added_sources.push(s);
                }
            }
        }

        if added_sources.is_empty() {
            return Err(crate::error::AppError::Config(format!(
                "目录 {} 中的配置文件均已作为独立配置源登记过，无需重复添加", target
            )));
        }

        let count = state.sync_nodes_and_users(false).await?;
        let _ = state.apply_and_reload().await;
        *state.status_cache.lock().await = None;

        return Ok(Json(json!({
            "success": true,
            "message": format!("已成功从目录中扫描并分别独立登记 {} 个配置文件源", added_sources.len()),
            "count": added_sources.len(),
            "data": added_sources,
            "total_nodes": count
        })));
    }

    let id = format!("src-{}", rand::random::<u32>());
    let source = StoredSource {
        id: id.clone(),
        name,
        source_type: payload.source_type,
        target,
        secret,
        enabled: payload.enabled,
        created_at: Utc::now(),
    };

    state.db.save_source(&source).await?;

    // Auto-sync after adding and apply hot reload
    let count = state.sync_nodes_and_users(false).await?;
    let _ = state.apply_and_reload().await;
    *state.status_cache.lock().await = None;

    Ok(Json(json!({
        "success": true,
        "message": format!("已成功登记配置源「{}」并完成热重载", source.name),
        "data": source,
        "total_nodes": count
    })))
}

pub async fn toggle_source(
    Path(id): Path<String>,
    State(state): State<SharedState>,
    Json(payload): Json<ToggleSourceRequest>,
) -> Result<Json<Value>> {
    state.db.toggle_source(&id, payload.enabled).await?;

    // Auto-sync after toggling and apply hot reload to Mihomo immediately
    let count = state.sync_nodes_and_users(false).await?;
    let _ = state.apply_and_reload().await;
    *state.status_cache.lock().await = None;

    Ok(Json(json!({
        "success": true,
        "message": format!("Source status updated to {}", if payload.enabled { "enabled" } else { "disabled" }),
        "total_nodes": count
    })))
}

pub async fn delete_source(
    Path(id): Path<String>,
    State(state): State<SharedState>,
) -> Result<Json<Value>> {
    state.db.delete_source(&id).await?;

    // Auto-sync after deleting and apply hot reload
    let count = state.sync_nodes_and_users(false).await?;
    let _ = state.apply_and_reload().await;
    *state.status_cache.lock().await = None;

    Ok(Json(json!({
        "success": true,
        "message": "Source removed successfully",
        "total_nodes": count
    })))
}

pub async fn update_source(
    Path(id): Path<String>,
    State(state): State<SharedState>,
    Json(payload): Json<UpdateSourceRequest>,
) -> Result<Json<Value>> {
    let name_trimmed = payload.name.as_deref().map(|n| n.trim());
    if let Some(n) = name_trimmed {
        if n.is_empty() {
            return Err(crate::error::AppError::Config("配置源名称不能为空".to_string()));
        }
    }

    let target_trimmed = payload.target.as_deref().map(|t| t.trim());
    if let Some(t) = target_trimmed {
        if t.is_empty() {
            return Err(crate::error::AppError::Config("目标路径或接口地址不能为空".to_string()));
        }
    }

    let secret_opt: Option<Option<&str>> = match payload.secret.as_deref() {
        Some(s) if s.trim().is_empty() => Some(None),
        Some(s) => Some(Some(s.trim())),
        None => None,
    };

    let updated = state.db.update_source(
        &id,
        name_trimmed,
        target_trimmed,
        secret_opt,
        payload.enabled,
    ).await?;

    if let Some(src) = updated {
        let count = state.sync_nodes_and_users(false).await?;
        let _ = state.apply_and_reload().await;
        *state.status_cache.lock().await = None;

        Ok(Json(json!({
            "success": true,
            "message": "Source updated successfully",
            "data": src,
            "total_nodes": count
        })))
    } else {
        Err(crate::error::AppError::Config(format!("Source {} not found", id)))
    }
}
