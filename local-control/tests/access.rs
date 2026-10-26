use meetily_local_control::gateway::*;
use meetily_local_control::ControlError;
use serde_json::{json,Value};
use std::sync::{Arc,atomic::{AtomicUsize,Ordering}};
use axum::{body::Body,http::Request};
use tower::ServiceExt;
struct Handler(AtomicUsize);
#[async_trait::async_trait]
impl Dispatcher for Handler { async fn dispatch(&self,_:&str,_:Value)->Result<Value,ControlError> { self.0.fetch_add(1,Ordering::SeqCst); Ok(json!({"ok":true})) } }
async fn call(router:axum::Router, token:&str,method:&str,origin:bool)->u16 {
 let mut r=Request::builder().method("POST").uri("/v1/rpc").header("host","127.0.0.1:4321").header("content-type","application/json").header("authorization",format!("Bearer {token}"));
 if origin { r=r.header("origin","https://evil.example"); }
 router.oneshot(r.body(Body::from(json!({"method":method,"params":{}}).to_string())).unwrap()).await.unwrap().status().as_u16()
}
#[tokio::test]
async fn browser_revoked_disabled_and_read_only_requests_never_dispatch() {
 let h=Arc::new(Handler(AtomicUsize::new(0)));
 let auth=Arc::new(tokio::sync::RwLock::new(Credentials{enabled:true,read_token:"read".into(),control_token:Some("control".into())}));
 let r=router(h.clone(),auth.clone(),4321);
 assert_eq!(call(r.clone(),"read","status",true).await,403);
 assert_eq!(call(r.clone(),"bad","status",false).await,401);
 assert_eq!(call(r.clone(),"read","recording.start",false).await,403);
 assert_eq!(h.0.load(Ordering::SeqCst),0);
 assert_eq!(call(r.clone(),"read","status",false).await,200);
 assert_eq!(call(r.clone(),"control","recording.start",false).await,200);
 auth.write().await.control_token=None;
 assert_eq!(call(r.clone(),"control","recording.start",false).await,401);
 auth.write().await.enabled=false;
 assert_eq!(call(r,"read","status",false).await,403);
 assert_eq!(h.0.load(Ordering::SeqCst),2);
}
#[tokio::test]
async fn rejects_rebinding_and_unknown_methods_and_malformed_json() {
 let h=Arc::new(Handler(AtomicUsize::new(0)));
 let auth=Arc::new(tokio::sync::RwLock::new(Credentials{enabled:true,read_token:"read".into(),control_token:None}));
 let r=router(h.clone(),auth,4321);
 let req=Request::builder().method("POST").uri("/v1/rpc").header("host","attacker.test:4321").header("content-type","application/json").header("authorization","Bearer read").body(Body::from(r#"{"method":"status","params":{}}"#)).unwrap();
 assert_eq!(r.clone().oneshot(req).await.unwrap().status().as_u16(),403);
 assert_eq!(call(r.clone(),"read","arbitrary.method",false).await,404);
 let req=Request::builder().method("POST").uri("/v1/rpc").header("host","127.0.0.1:4321").header("content-type","application/json").header("authorization","Bearer read").body(Body::from("invalid JSON")).unwrap();
 assert_eq!(r.oneshot(req).await.unwrap().status().as_u16(),400);
 assert_eq!(h.0.load(Ordering::SeqCst),0);
}
#[tokio::test]
async fn cancelling_http_wait_does_not_cancel_accepted_operation() {
 struct Slow{started:Arc<tokio::sync::Notify>,finish:Arc<tokio::sync::Notify>,done:Arc<AtomicUsize>}
 #[async_trait::async_trait]
 impl Dispatcher for Slow {async fn dispatch(&self,_:&str,_:Value)->Result<Value,ControlError>{self.started.notify_one();self.finish.notified().await;self.done.store(1,Ordering::SeqCst);Ok(json!({}))}}
 let started=Arc::new(tokio::sync::Notify::new());let finish=Arc::new(tokio::sync::Notify::new());let done=Arc::new(AtomicUsize::new(0));
 let r=router(Arc::new(Slow{started:started.clone(),finish:finish.clone(),done:done.clone()}),Arc::new(tokio::sync::RwLock::new(Credentials{enabled:true,read_token:"read".into(),control_token:Some("control".into())})),4321);
 let task=tokio::spawn(async move{call(r,"control","recording.start",false).await});
 started.notified().await;task.abort();finish.notify_one();
 tokio::time::timeout(std::time::Duration::from_secs(1),async {while done.load(Ordering::SeqCst)==0{tokio::task::yield_now().await}}).await.unwrap();
}
