pub mod dispatcher;
pub mod methods;

#[derive(Debug, Clone)]
pub struct SessionContext {
    pub conn_id: String,
    pub role: String,
    pub scopes: Vec<String>,
    pub client_id: String,
    pub client_mode: String,
}
