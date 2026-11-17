//! Observe rgctl extraction diagnostics without reparsing or changing its facts.
use rgctl_plugin_api::*;
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::Mutex,
};

pub struct ObservedJava {
    plugin: rgctl_lang_java::JavaPlugin,
    pub ambiguities: Mutex<HashSet<(String, String)>>,
}
impl ObservedJava {
    pub fn new() -> Result<Self> {
        Ok(Self {
            plugin: rgctl_lang_java::JavaPlugin::new()?,
            ambiguities: Mutex::new(HashSet::new()),
        })
    }
}
impl LanguagePlugin for ObservedJava {
    fn language_id(&self) -> &str {
        self.plugin.language_id()
    }
    fn file_extensions(&self) -> Vec<&str> {
        self.plugin.file_extensions()
    }
    fn grammar(&self) -> Option<tree_sitter::Language> {
        self.plugin.grammar()
    }
    fn capabilities(&self) -> LanguageCapabilities {
        self.plugin.capabilities()
    }
    fn declared_ast_handlers(&self) -> Option<&AstHandlerTable> {
        self.plugin.declared_ast_handlers()
    }
    fn cache_fingerprint(&self) -> Option<&[u8]> {
        self.plugin.cache_fingerprint()
    }
    fn extract_symbols(&self, path: &Path, source: &[u8]) -> Result<Vec<Symbol>> {
        self.plugin.extract_symbols(path, source)
    }
    fn extract_relations(
        &self,
        path: &Path,
        source: &[u8],
        symbols: &[Symbol],
    ) -> Result<Vec<Relation>> {
        self.plugin.extract_relations(path, source, symbols)
    }
    fn calculate_complexity(
        &self,
        symbol: &Symbol,
        source: &[u8],
    ) -> Result<Option<ComplexityMetrics>> {
        self.plugin.calculate_complexity(symbol, source)
    }
    fn extract_all(&self, path: &Path, source: &[u8]) -> Result<ExtractAllResult> {
        let result = self.plugin.extract_all(path, source)?;
        let mut names = HashMap::new();
        for symbol in &result.symbols {
            if !matches!(
                symbol.symbol_type,
                SymbolType::Function | SymbolType::Method
            ) {
                continue;
            }
            let name = symbol.qualified_name.as_deref().unwrap_or(&symbol.name);
            let discriminator = symbol
                .metadata
                .get("identity_discriminator")
                .and_then(|v| v.as_str());
            if names
                .insert((name, discriminator), symbol.signature.as_deref())
                .is_some()
            {
                if let Ok(mut ambiguous) = self.ambiguities.lock() {
                    ambiguous.insert((path.to_string_lossy().into_owned(), name.into()));
                }
            }
        }
        Ok(result)
    }
}
