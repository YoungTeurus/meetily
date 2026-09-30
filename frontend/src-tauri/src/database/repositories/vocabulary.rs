use sqlx::SqlitePool;
use std::collections::HashSet;

pub const MAX_VOCABULARY_CHARS: usize = 1000;
pub struct VocabularyRepository;
impl VocabularyRepository {
    pub fn normalize(raw: &str) -> Result<Option<String>, String> {
        if raw.contains('\0') {
            return Err("Vocabulary cannot contain null characters".into());
        }
        let mut seen = HashSet::new();
        let terms: Vec<_> = raw
            .split([',', '\n', '\r'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .filter(|s| seen.insert(s.to_lowercase()))
            .collect();
        let text = terms.join("\n");
        if text.chars().count() > MAX_VOCABULARY_CHARS {
            return Err(format!(
                "Vocabulary must be {MAX_VOCABULARY_CHARS} characters or fewer"
            ));
        }
        Ok((!text.is_empty()).then_some(text))
    }
    /// Run-specific terms come first because Whisper has a finite prompt budget.
    pub fn merge(run: Option<&str>, global: Option<&str>) -> Option<String> {
        let mut seen = HashSet::new();
        let terms: Vec<_> = [run, global]
            .into_iter()
            .flatten()
            .flat_map(|s| s.split([',', '\n', '\r']))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .filter(|s| seen.insert(s.to_lowercase()))
            .collect();
        (!terms.is_empty()).then(|| terms.join(", "))
    }
    pub async fn get_global(pool: &SqlitePool) -> Result<Option<String>, sqlx::Error> {
        sqlx::query_scalar("SELECT vocabulary FROM transcription_vocabulary WHERE id='global'")
            .fetch_optional(pool)
            .await
    }
    pub async fn save_global(
        pool: &SqlitePool,
        raw: Option<&str>,
    ) -> Result<Option<String>, String> {
        let value = Self::normalize(raw.unwrap_or_default())?;
        match value.as_deref() {
            Some(v) => {
                sqlx::query("INSERT INTO transcription_vocabulary(id,vocabulary) VALUES('global',?) ON CONFLICT(id) DO UPDATE SET vocabulary=excluded.vocabulary").bind(v).execute(pool).await.map_err(|e|e.to_string())?;
            }
            None => {
                sqlx::query("DELETE FROM transcription_vocabulary WHERE id='global'")
                    .execute(pool)
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(value)
    }
}
