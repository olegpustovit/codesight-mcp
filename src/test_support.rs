//! Synthetic workspaces and checked-in fixture paths for integration tests.
//! Used by integration tests; not part of the MCP runtime.

use std::fs;
use std::path::Path;

const SMALL_UTILS_PY: &str = r#"
MAX_RETRIES = 3

def add(a, b):
    return a + b

def subtract(a, b):
    return a - b
"#;

const MEDIUM_MODELS_PY: &str = r#"
class User:
    """Represents a user."""
    pass

class Product:
    """Represents a product."""
    pass
"#;

const MEDIUM_SERVICE_PY: &str = r#"
from models import User

def get_user(user_id):
    return User()

def create_user(name):
    return User()
"#;

const MEDIUM_API_PY: &str = r#"
from models import User, Product
from service import get_user

def handle_request(req):
    return get_user(req)
"#;

const HIERARCHY_ANIMALS_PY: &str = r#"
class Animal:
    """Base animal class."""
    pass

class Mammal(Animal):
    """A warm-blooded animal."""
    pass

class Dog(Mammal):
    """A domestic dog."""
    pass

class Cat(Mammal):
    """A domestic cat."""
    pass
"#;

/// One file, three symbols: `MAX_RETRIES`, `add`, `subtract`.
pub fn write_small_repo(src: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(src)?;
    fs::write(src.join("utils.py"), SMALL_UTILS_PY.trim_start())?;
    Ok(())
}

/// models.py / service.py / api.py with cross-imports.
pub fn write_medium_repo(src: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(src)?;
    fs::write(src.join("models.py"), MEDIUM_MODELS_PY.trim_start())?;
    fs::write(src.join("service.py"), MEDIUM_SERVICE_PY.trim_start())?;
    fs::write(src.join("api.py"), MEDIUM_API_PY.trim_start())?;
    Ok(())
}

/// Animal → Mammal → Dog / Cat.
pub fn write_hierarchy_repo(src: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(src)?;
    fs::write(src.join("animals.py"), HIERARCHY_ANIMALS_PY.trim_start())?;
    Ok(())
}

/// Root of the checked-in documentation fixture tree under `tests/fixtures/doc_reference/`.
pub fn doc_reference_fixtures_dir() -> std::path::PathBuf {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("tests/fixtures/doc_reference")
}

/// Root of the checked-in code fixture tree under `tests/fixtures/code_reference/`.
pub fn code_reference_fixtures_dir() -> std::path::PathBuf {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("tests/fixtures/code_reference")
}
