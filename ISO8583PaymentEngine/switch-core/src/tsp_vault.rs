use dashmap::DashMap;

pub struct TspVault {
    pub token_map: DashMap<String, String>,
}

impl TspVault {
    pub fn new() -> Self {
        let vault = Self {
            token_map: DashMap::new(),
        };
        // Seed test token
        vault.token_map.insert("4000000000009999".to_string(), "4111222233334444".to_string());
        vault
    }
    
    pub fn detokenize(&self, dpan: &str) -> Option<String> {
        self.token_map.get(dpan).map(|v| v.clone())
    }
}
