//! Fixture paths and synthetic-repo writers for the parser test suite.

use std::fs;
use std::path::{Path, PathBuf};

pub fn code_reference_fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/code_reference")
}

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

pub fn write_small_repo(src: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(src)?;
    fs::write(src.join("utils.py"), SMALL_UTILS_PY.trim_start())?;
    Ok(())
}

pub fn write_medium_repo(src: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(src)?;
    fs::write(src.join("models.py"), MEDIUM_MODELS_PY.trim_start())?;
    fs::write(src.join("service.py"), MEDIUM_SERVICE_PY.trim_start())?;
    fs::write(src.join("api.py"), MEDIUM_API_PY.trim_start())?;
    Ok(())
}

pub fn write_hierarchy_repo(src: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(src)?;
    fs::write(src.join("animals.py"), HIERARCHY_ANIMALS_PY.trim_start())?;
    Ok(())
}
