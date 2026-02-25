pub mod dispatcher;
pub mod methods;
pub mod policy;

#[derive(Debug, Clone)]
pub struct SessionContext {
    pub conn_id: String,
    pub role: String,
    pub scopes: Vec<String>,
    pub client_id: String,
    pub client_mode: String,
}
