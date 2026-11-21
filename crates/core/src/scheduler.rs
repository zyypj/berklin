use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use chrono::{Local, TimeZone};

use crate::agent::Agent;
use crate::model::{now_ms, one_line, Schedule};
use crate::runner;

const DOW_NAMES: [&str; 8] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT", "SUN"];

fn unix_dow_to_names(field: &str) -> String {
    field
        .split(',')
        .map(|part| {
            let (base, step) = match part.split_once('/') {
                Some((b, s)) => (b, Some(s)),
                None => (part, None),
            };
            let base = base
                .split('-')
                .map(|n| n.parse::<usize>().ok().and_then(|i| DOW_NAMES.get(i)).map(|s| s.to_string()).unwrap_or_else(|| n.to_string()))
                .collect::<Vec<_>>()
                .join("-");
            match step {
                Some(s) => format!("{base}/{s}"),
                None => base,
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn cron_next(expr: &str, after_ms: i64) -> Result<Option<i64>, String> {
    let parts: Vec<&str> = expr.split_whitespace().collect();
    let full = match parts.len() {
        5 => format!("0 {} {} {} {} {}", parts[0], parts[1], parts[2], parts[3], unix_dow_to_names(parts[4])),
        6 | 7 => expr.to_string(),
        _ => return Err("cron precisa de 5 campos: minuto hora dia mês dia-da-semana".into()),
    };
    let sched = cron::Schedule::from_str(&full).map_err(|e| format!("cron inválido: {e}"))?;
    let after = Local.timestamp_millis_opt(after_ms).single().unwrap_or_else(Local::now);
    Ok(sched.after(&after).next().map(|d| d.timestamp_millis()))
}

fn compute_next(s: &Schedule, after_ms: i64) -> Result<Option<i64>, String> {
    if !s.enabled {
        return Ok(None);
    }
    if let Some(expr) = s.cron.as_deref().filter(|c| !c.trim().is_empty()) {
        return cron_next(expr.trim(), after_ms);
    }
    Ok(s.at)
}

fn persist(agent: &Agent, list: &[Schedule]) {
    if let Ok(b) = serde_json::to_vec_pretty(list) {
        let path = agent.paths.schedules();
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, b).is_ok() {
            let _ = std::fs::rename(tmp, path);
        }
    }
}

pub fn save(agent: &Arc<Agent>, mut s: Schedule) -> Result<Schedule, String> {
    if s.prompt.trim().is_empty() {
        return Err("Escreva o que deve ser feito.".into());
    }
    if s.kind != "reminder" {
        s.kind = "claude".into();
        if s.cwd.trim().is_empty() || !std::path::Path::new(s.cwd.trim()).is_dir() {
            return Err("Escolha uma pasta válida para o Claude trabalhar.".into());
        }
    }
    if s.cron.as_deref().map(|c| c.trim().is_empty()).unwrap_or(true) {
        s.cron = None;
        if s.at.is_none() {
            return Err("Defina a data/hora ou a recorrência.".into());
        }
    }
    if s.name.trim().is_empty() {
        s.name = one_line(&s.prompt, 40);
    }
    if s.id.is_empty() {
        s.id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
    }
    s.next_run = compute_next(&s, now_ms())?;
    if s.enabled && s.next_run.is_none() {
        return Err("Essa recorrência nunca acontece.".into());
    }
    {
        let mut inner = agent.inner.lock();
        if let Some(existing) = inner.schedules.iter_mut().find(|x| x.id == s.id) {
            s.last_run = existing.last_run;
            s.last_status = existing.last_status.clone();
            s.last_sid = existing.last_sid.clone();
            *existing = s.clone();
        } else {
            inner.schedules.push(s.clone());
        }
        persist(agent, &inner.schedules);
    }
    agent.emit_schedule(&s);
    agent.sched_wake.notify_one();
    Ok(s)
}

pub fn delete(agent: &Arc<Agent>, id: &str) {
    {
        let mut inner = agent.inner.lock();
        inner.schedules.retain(|s| s.id != id);
        persist(agent, &inner.schedules);
    }
    agent.emit_schedule_gone(id);
    agent.sched_wake.notify_one();
}

pub fn mark(agent: &Agent, id: &str, status: &str, sid: Option<&str>) {
    let updated = {
        let mut inner = agent.inner.lock();
        let found = inner.schedules.iter_mut().find(|s| s.id == id).map(|s| {
            s.last_status = Some(status.to_string());
            if let Some(sid) = sid {
                s.last_sid = Some(sid.to_string());
            }
            s.clone()
        });
        if found.is_some() {
            persist(agent, &inner.schedules);
        }
        found
    };
    if let Some(s) = updated {
        agent.emit_schedule(&s);
    }
}

pub async fn run_now(agent: &Arc<Agent>, id: &str) -> Result<(), String> {
    let s = agent.inner.lock().schedules.iter().find(|s| s.id == id).cloned().ok_or("Agendamento não encontrado.")?;
    fire(agent, s).await;
    Ok(())
}

async fn fire(agent: &Arc<Agent>, s: Schedule) {
    if s.kind == "reminder" {
        mark(agent, &s.id, "ok", None);
        agent.note("schedule", format!("Lembrete: {}", s.name), s.prompt.clone(), None, None);
        return;
    }
    mark(agent, &s.id, "rodando", None);
    if s.notify != "claude" {
        agent.note("schedule", format!("Agendamento iniciado: {}", s.name), one_line(&s.prompt, 200), None, None);
    }
    match runner::start(agent, None, s.cwd.clone(), s.prompt.clone(), s.mode.clone(), Some(s.id.clone()), s.chrome).await {
        Ok(sid) => mark(agent, &s.id, "rodando", Some(&sid)),
        Err(e) => {
            mark(agent, &s.id, "erro", None);
            agent.note("error", format!("Agendamento falhou: {}", s.name), e, None, None);
        }
    }
}

pub async fn run(agent: Arc<Agent>) {
    {
        let mut inner = agent.inner.lock();
        let now = now_ms();
        for s in inner.schedules.iter_mut() {
            s.next_run = compute_next(s, now).ok().flatten();
            if s.cron.is_none() && s.at.map(|a| a < now - 60_000).unwrap_or(false) && s.enabled {
                s.next_run = Some(now);
            }
        }
        persist(&agent, &inner.schedules);
    }
    loop {
        let now = now_ms();
        let due: Vec<Schedule> = {
            let mut inner = agent.inner.lock();
            let mut due = Vec::new();
            for s in inner.schedules.iter_mut() {
                if s.enabled && s.next_run.map(|n| n <= now).unwrap_or(false) {
                    s.last_run = Some(now);
                    if s.cron.is_none() {
                        s.enabled = false;
                        s.next_run = None;
                    } else {
                        s.next_run = compute_next(s, now + 1000).ok().flatten();
                    }
                    due.push(s.clone());
                }
            }
            if !due.is_empty() {
                persist(&agent, &inner.schedules);
            }
            due
        };
        for s in due {
            agent.emit_schedule(&s);
            let a = agent.clone();
            tokio::spawn(async move { fire(&a, s).await });
        }
        let next = agent.inner.lock().schedules.iter().filter_map(|s| s.next_run).min();
        let wait = next.map(|n| (n - now_ms()).clamp(200, 30_000)).unwrap_or(30_000);
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(wait as u64)) => {}
            _ = agent.sched_wake.notified() => {}
        }
    }
}
