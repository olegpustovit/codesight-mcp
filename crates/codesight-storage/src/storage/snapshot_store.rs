//! Quality snapshot persistence: save/load named quality baselines.

use anyhow::Context;

use super::IndexStore;

/// A persisted quality snapshot for regression comparison.
#[derive(Debug, Clone)]
pub struct QualitySnapshot {
    pub source_id: String,
    pub name: String,
    pub created_at: String,
    pub score: u32,
    pub structural_score: u32,
    pub agent_readiness_score: u32,
    pub modularity: Option<f64>,
    pub acyclicity: Option<f64>,
    pub depth: Option<f64>,
    pub equality: Option<f64>,
    pub redundancy: Option<f64>,
    pub navigability: Option<f64>,
    pub comprehensibility: Option<f64>,
    pub verifiability: Option<f64>,
    pub change_safety: Option<f64>,
    pub cycle_count: u32,
    pub dead_count: u32,
    pub dup_count: u32,
    pub file_count: u32,
    pub symbol_count: u32,
    pub confidence: String,
    pub avg_complexity: f64,
}

impl IndexStore {
    /// Save (or overwrite) a named quality snapshot.
    pub fn save_snapshot(&self, snap: &QualitySnapshot) -> anyhow::Result<()> {
        let conn = self.open_connection()?;
        conn.execute(
            "INSERT OR REPLACE INTO quality_snapshots (
                source_id, name, score, structural_score, agent_readiness_score,
                modularity, acyclicity, depth, equality, redundancy,
                navigability, comprehensibility, verifiability, change_safety,
                cycle_count, dead_count, dup_count, file_count, symbol_count,
                confidence, avg_complexity
            ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21)",
            rusqlite::params![
                snap.source_id,
                snap.name,
                snap.score,
                snap.structural_score,
                snap.agent_readiness_score,
                snap.modularity,
                snap.acyclicity,
                snap.depth,
                snap.equality,
                snap.redundancy,
                snap.navigability,
                snap.comprehensibility,
                snap.verifiability,
                snap.change_safety,
                snap.cycle_count,
                snap.dead_count,
                snap.dup_count,
                snap.file_count,
                snap.symbol_count,
                snap.confidence,
                snap.avg_complexity,
            ],
        )
        .context("save quality snapshot")?;
        Ok(())
    }

    /// Load a named quality snapshot. Returns `None` if no snapshot exists.
    pub fn load_snapshot(
        &self,
        source_id: &str,
        name: &str,
    ) -> anyhow::Result<Option<QualitySnapshot>> {
        let conn = self.open_read_only()?;
        let mut stmt = conn.prepare(
            "SELECT source_id, name, created_at, score, structural_score,
                    agent_readiness_score, modularity, acyclicity, depth,
                    equality, redundancy, navigability, comprehensibility,
                    verifiability, change_safety, cycle_count, dead_count,
                    dup_count, file_count, symbol_count, confidence, avg_complexity
             FROM quality_snapshots
             WHERE source_id = ?1 AND name = ?2",
        )?;
        let result = stmt.query_row(rusqlite::params![source_id, name], |row| {
            Ok(QualitySnapshot {
                source_id: row.get(0)?,
                name: row.get(1)?,
                created_at: row.get(2)?,
                score: row.get(3)?,
                structural_score: row.get(4)?,
                agent_readiness_score: row.get(5)?,
                modularity: row.get(6)?,
                acyclicity: row.get(7)?,
                depth: row.get(8)?,
                equality: row.get(9)?,
                redundancy: row.get(10)?,
                navigability: row.get(11)?,
                comprehensibility: row.get(12)?,
                verifiability: row.get(13)?,
                change_safety: row.get(14)?,
                cycle_count: row.get(15)?,
                dead_count: row.get(16)?,
                dup_count: row.get(17)?,
                file_count: row.get(18)?,
                symbol_count: row.get(19)?,
                confidence: row.get(20)?,
                avg_complexity: row.get(21)?,
            })
        });
        match result {
            Ok(snap) => Ok(Some(snap)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}
