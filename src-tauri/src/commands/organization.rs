//! Narrow ID-only IPC for reviewed organization. Native picker owns paths.
use super::{AppState, Operation};
use crate::organization::{
    self as org, Batch, Destination, Grouping, Inventory, Kind, Plan, Preference,
};
use crate::{Result, ScuttleError};
use std::sync::{atomic::AtomicBool, Arc};
use tauri::State;

#[tauri::command]
pub async fn organization_inventory(state: State<'_, AppState>) -> Result<Inventory> {
    let state = state.clone_handle();
    tauri::async_runtime::spawn_blocking(move || org::inventory(&state))
        .await
        .map_err(|_| ScuttleError::Internal("Could not load organization opportunities.".into()))?
}
#[tauri::command]
pub async fn organization_preference(state: State<'_, AppState>, kind: Kind) -> Result<Preference> {
    super::gated(&state, Operation::Refresh, move |state| {
        org::preference(state, kind)
    })
    .await
}
#[tauri::command]
pub async fn choose_organization_destination(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<Destination>> {
    use tauri_plugin_dialog::DialogExt;
    let state = state.clone_handle();
    tauri::async_runtime::spawn_blocking(move || {
        let picked = app
            .dialog()
            .file()
            .set_title("Give these files a home")
            .blocking_pick_folder();
        picked
            .map(|p| {
                let path = p
                    .into_path()
                    .map_err(|_| ScuttleError::Refused("Choose a local folder.".into()))?;
                org::register_destination(&state, path)
            })
            .transpose()
    })
    .await
    .map_err(|_| ScuttleError::Internal("Could not open the folder picker.".into()))?
}
#[tauri::command]
pub async fn plan_organization(
    state: State<'_, AppState>,
    kind: Kind,
    ids: Vec<String>,
    destination_id: String,
    grouping: Grouping,
) -> Result<Plan> {
    super::gated(&state, Operation::Refresh, move |state| {
        org::plan(state, kind, ids, &destination_id, grouping)
    })
    .await
}

async fn launch(state: &AppState, request_id: String, undo: bool) -> Result<Batch> {
    let guard = state.begin_operation(Operation::Organize)?;
    let cancel = Arc::new(AtomicBool::new(false));
    *state
        .organization()
        .active
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some((request_id.clone(), cancel.clone()));
    let handle = state.clone_handle();
    let prepared = tauri::async_runtime::spawn_blocking(move || {
        let batch = org::reconcile(&handle).and_then(|()| {
            if undo {
                org::prepare_undo(&handle, &request_id)
            } else {
                org::create_batch(&handle, &request_id)
            }
        });
        (handle, guard, batch)
    })
    .await;
    let (handle, guard, result) = match prepared {
        Ok(value) => value,
        Err(_) => {
            *state
                .organization()
                .active
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = None;
            return Err(ScuttleError::Internal(
                "Could not prepare the organization job.".into(),
            ));
        }
    };
    let mut batch = match result {
        Ok(batch) => batch,
        Err(error) => {
            *state
                .organization()
                .active
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = None;
            return Err(error);
        }
    };
    *state
        .organization()
        .active
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some((batch.id.clone(), cancel.clone()));
    let initial = batch.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        struct Clear(AppState);
        impl Drop for Clear {
            fn drop(&mut self) {
                *self
                    .0
                    .organization()
                    .active
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = None;
            }
        }
        let _clear = Clear(handle.clone_handle());
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            org::run(&handle, &mut batch, &cancel)
        }));
        let error = match outcome {
            Ok(Ok(())) => None,
            Ok(Err(e)) => Some(e.to_string()),
            Err(_) => Some(
                "Organization stopped unexpectedly. Check the results before trying again.".into(),
            ),
        };
        if let Some(error) = error {
            let _ = org::reconcile(&handle);
            if let Ok(mut saved) = org::batch(&handle, &batch.id) {
                saved.error = Some(error);
                saved.running = false;
                saved.revision += 1;
                let _ = handle.store().organization_put("batch", &saved.id, &saved);
            }
        }
    });
    Ok(initial)
}
#[tauri::command]
pub async fn start_organization(state: State<'_, AppState>, plan_id: String) -> Result<Batch> {
    launch(&state, plan_id, false).await
}
#[tauri::command]
pub async fn undo_organization(state: State<'_, AppState>, id: String) -> Result<Batch> {
    launch(&state, id, true).await
}
#[tauri::command]
pub async fn organization_status(state: State<'_, AppState>, id: String) -> Result<Batch> {
    let state = state.clone_handle();
    tauri::async_runtime::spawn_blocking(move || org::batch(&state, &id))
        .await
        .map_err(|_| ScuttleError::Internal("Could not read the organization job.".into()))?
}
#[tauri::command]
pub async fn organization_history(state: State<'_, AppState>, offset: usize) -> Result<Vec<Batch>> {
    let state = state.clone_handle();
    tauri::async_runtime::spawn_blocking(move || {
        state.store().organization_list("batch", offset, 20)
    })
    .await
    .map_err(|_| ScuttleError::Internal("Could not load organization history.".into()))?
}
#[tauri::command]
pub fn cancel_organization(state: State<'_, AppState>, id: String) {
    if let Some((active, cancel)) = state
        .organization()
        .active
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
    {
        if active == &id {
            cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
}
#[tauri::command]
pub async fn organization_thumbnail(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<String>> {
    let state = state.clone_handle();
    tauri::async_runtime::spawn_blocking(move || org::thumbnail(&state, &id))
        .await
        .map_err(|_| ScuttleError::Internal("Could not load this preview.".into()))?
}
#[tauri::command]
pub async fn reveal_organization(state: State<'_, AppState>, id: String) -> Result<()> {
    let state = state.clone_handle();
    tauri::async_runtime::spawn_blocking(move || org::open_folder(&state, &id))
        .await
        .map_err(|_| ScuttleError::Internal("Could not open the folder.".into()))?
}
