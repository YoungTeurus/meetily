use meetily_local_control::import::snapshot_for_import;
use sqlx::{Connection,sqlite::SqliteConnectOptions};
#[tokio::test]
async fn refuses_newer_schema_and_snapshots_wal_without_modifying_source(){
 let dir=tempfile::tempdir().unwrap();let source=dir.path().join("source.sqlite");let dest=dir.path().join("backup.sqlite");
 let mut db=sqlx::SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(&source).create_if_missing(true)).await.unwrap();
 sqlx::raw_sql("PRAGMA journal_mode=WAL; CREATE TABLE meetings(id TEXT,title TEXT,created_at TEXT,updated_at TEXT); CREATE TABLE transcripts(id TEXT,meeting_id TEXT,transcript TEXT,timestamp TEXT); CREATE TABLE _sqlx_migrations(version INTEGER); INSERT INTO _sqlx_migrations VALUES(99); INSERT INTO meetings VALUES('m','Test','2026-09-30','2026-09-30');").execute(&mut db).await.unwrap();
 assert!(snapshot_for_import(&source,&dest,98).await.is_err());assert!(!dest.exists());
 snapshot_for_import(&source,&dest,99).await.unwrap();
 let mut backup=sqlx::SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(&dest).read_only(true)).await.unwrap();
 let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM meetings").fetch_one(&mut backup).await.unwrap();assert_eq!(count,1);
 assert!(snapshot_for_import(&source,&dest,99).await.is_err());
}
