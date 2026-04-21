use super::*;

use codesight_core::symbols::SymbolKind;

fn make_extractor(key: &str) -> ScmExtractor {
    let source = query_files_slice()
        .iter()
        .find(|(candidate, _)| *candidate == key)
        .unwrap_or_else(|| panic!("no query file for {key}"))
        .1;
    let meta = parse_scm_header(source);
    ScmExtractor::new(&meta, source).unwrap()
}

#[test]
fn scm_bash_extracts_functions_and_constants() {
    let ext = make_extractor("bash");
    let src = r#"#!/bin/bash
MAX_RETRIES=3
TIMEOUT=30

greet() {
    echo "Hello $1"
}

function deploy {
    echo "deploying..."
}
"#;
    let symbols = ext.extract("deploy.sh", src).expect("bash");
    let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"greet"), "missing greet: {names:?}");
    assert!(names.contains(&"deploy"), "missing deploy: {names:?}");
    assert!(
        names.contains(&"MAX_RETRIES"),
        "missing MAX_RETRIES: {names:?}"
    );
}

#[test]
fn scm_java_classes_methods_constants() {
    let ext = make_extractor("java");
    let src = r#"
public class UserService {
    private static final int MAX_RETRIES = 3;

    public void createUser(String name) {}
    public String getUser(int id) { return null; }
}

public interface Repo { void save(); }
public enum Color { RED, GREEN; }
"#;
    let symbols = ext.extract("Test.java", src).expect("java");
    let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"UserService"), "missing UserService");
    assert!(names.contains(&"createUser"), "missing createUser");
    assert!(names.contains(&"getUser"), "missing getUser");
    assert!(names.contains(&"Repo"), "missing Repo");
    assert!(names.contains(&"Color"), "missing Color");

    let method = symbols.iter().find(|s| s.name == "createUser").unwrap();
    assert_eq!(method.kind, SymbolKind::Method);
    assert_eq!(method.qualified_name, "UserService.createUser");
}

#[test]
fn scm_python_decorators_docstrings_constants() {
    let ext = make_extractor("python");
    let src = r#"
MAX_RETRIES = 3

class UserService:
    """Handles user operations."""

    @classmethod
    def find_user(cls, user_id: str):
        """Find a user by id."""
        return user_id

def top_level(value):
    return value
"#;
    let symbols = ext.extract("svc.py", src).expect("python");
    let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"MAX_RETRIES"), "missing MAX_RETRIES");
    assert!(names.contains(&"UserService"), "missing UserService");
    assert!(names.contains(&"find_user"), "missing find_user");
    assert!(names.contains(&"top_level"), "missing top_level");

    let method = symbols.iter().find(|s| s.name == "find_user").unwrap();
    assert_eq!(method.kind, SymbolKind::Method);
    assert_eq!(method.qualified_name, "UserService.find_user");
    assert!(
        method.source_text.contains("@classmethod"),
        "decorator not in source_text"
    );
}

#[test]
fn scm_python_constants_excluded_inside_functions() {
    let ext = make_extractor("python");
    let src = r#"
TOP_LEVEL = 1

def func():
    INSIDE_FUNC = 2

class Cls:
    CLASS_CONST = 3
"#;
    let symbols = ext.extract("test.py", src).expect("python");
    let const_names: Vec<&str> = symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Constant)
        .map(|s| s.name.as_str())
        .collect();
    assert!(const_names.contains(&"TOP_LEVEL"), "missing TOP_LEVEL");
    assert!(const_names.contains(&"CLASS_CONST"), "missing CLASS_CONST");
    assert!(
        !const_names.contains(&"INSIDE_FUNC"),
        "INSIDE_FUNC should be excluded"
    );
}

#[test]
fn scm_go_receiver_scoping() {
    let ext = make_extractor("go");
    let src = r#"
package p

const Version = "1"

type Server struct { addr string }

func (s *Server) Addr() string { return s.addr }
func main() {}
"#;
    let symbols = ext.extract("main.go", src).expect("go");
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "Version" && s.kind == SymbolKind::Constant)
    );
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "Server" && s.kind == SymbolKind::Class)
    );
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "main" && s.kind == SymbolKind::Function)
    );

    let method = symbols.iter().find(|s| s.name == "Addr").unwrap();
    assert_eq!(method.kind, SymbolKind::Method);
    assert_eq!(method.qualified_name, "Server.Addr");
}

#[test]
fn scm_bash_query_compiles() {
    make_extractor("bash");
}

#[test]
fn scm_java_query_compiles() {
    make_extractor("java");
}

#[test]
fn scm_python_query_compiles() {
    make_extractor("python");
}

#[test]
fn scm_go_query_compiles() {
    make_extractor("go");
}

#[test]
fn scm_php_query_compiles() {
    make_extractor("php");
}

#[test]
fn scm_javascript_query_compiles() {
    make_extractor("javascript");
}

#[test]
fn scm_rust_query_compiles() {
    make_extractor("rust");
}

#[test]
fn scm_cpp_query_compiles() {
    make_extractor("cpp");
}

#[test]
fn scm_csharp_query_compiles() {
    make_extractor("csharp");
}

#[test]
fn scm_typescript_query_compiles() {
    make_extractor("typescript");
}

#[test]
fn scm_tsx_query_compiles() {
    make_extractor("tsx");
}

#[test]
fn scm_json_query_compiles() {
    make_extractor("json");
}

#[test]
fn scm_css_query_compiles() {
    make_extractor("css");
}

#[test]
fn scm_ruby_query_compiles() {
    make_extractor("ruby");
}

#[test]
fn scm_lua_query_compiles() {
    make_extractor("lua");
}

#[test]
fn scm_swift_query_compiles() {
    make_extractor("swift");
}

#[test]
fn scm_perl_query_compiles() {
    make_extractor("perl");
}

#[test]
fn scm_elixir_query_compiles() {
    make_extractor("elixir");
}

#[test]
fn scm_erlang_query_compiles() {
    make_extractor("erlang");
}

#[test]
fn scm_kotlin_query_compiles() {
    make_extractor("kotlin");
}

#[test]
fn scm_dart_query_compiles() {
    make_extractor("dart");
}

#[test]
fn scm_xml_query_compiles() {
    make_extractor("xml");
}

#[test]
fn scm_nix_query_compiles() {
    make_extractor("nix");
}

#[test]
fn scm_fortran_query_compiles() {
    make_extractor("fortran");
}

#[test]
fn scm_gleam_query_compiles() {
    make_extractor("gleam");
}

#[test]
fn scm_all_queries_compile() {
    let extractors = build_scm_extractors().expect("build_scm_extractors failed");
    assert!(!extractors.is_empty(), "no extractors built");
}

#[test]
fn scm_bash_full_coverage() {
    let src = "#!/bin/bash\nMAX_RETRIES=3\ngreet() { echo hi; }\nfunction deploy { echo go; }";
    let scm = make_extractor("bash").extract("t.sh", src).unwrap();
    let names: Vec<(&str, &str)> = scm
        .iter()
        .map(|s| (s.name.as_str(), s.kind.as_str()))
        .collect();
    assert!(
        names.contains(&("greet", "function")),
        "missing greet: {names:?}"
    );
    assert!(
        names.contains(&("deploy", "function")),
        "missing deploy: {names:?}"
    );
    assert!(
        names.contains(&("MAX_RETRIES", "constant")),
        "missing MAX_RETRIES: {names:?}"
    );
}

#[test]
fn scm_java_full_coverage() {
    let src = r#"
public class Svc {
    private static final int MAX = 3;
    public void run() {}
    public Svc() {}
}
public interface Repo { void save(); }
public enum Color { RED; }
"#;
    let scm = make_extractor("java").extract("T.java", src).unwrap();
    let names: Vec<(&str, &str)> = scm
        .iter()
        .map(|s| (s.name.as_str(), s.kind.as_str()))
        .collect();
    assert!(names.contains(&("Svc", "class")), "missing Svc: {names:?}");
    assert!(names.contains(&("run", "method")), "missing run: {names:?}");
    assert!(names.contains(&("Repo", "type")), "missing Repo: {names:?}");
    assert!(
        names.contains(&("Color", "type")),
        "missing Color: {names:?}"
    );
}

#[test]
fn scm_python_full_coverage() {
    let src = r#"
MAX = 3
class Svc:
    """Doc."""
    @deco
    def method(self):
        """Method doc."""
        pass
def top(): pass
"#;
    let scm = make_extractor("python").extract("s.py", src).unwrap();
    let names: Vec<(&str, &str)> = scm
        .iter()
        .map(|s| (s.name.as_str(), s.kind.as_str()))
        .collect();
    assert!(
        names.contains(&("MAX", "constant")),
        "missing MAX: {names:?}"
    );
    assert!(names.contains(&("Svc", "class")), "missing Svc: {names:?}");
    assert!(
        names.contains(&("method", "method")),
        "missing method: {names:?}"
    );
    assert!(
        names.contains(&("top", "function")),
        "missing top: {names:?}"
    );
}

#[test]
fn scm_go_full_coverage() {
    let src = r#"
package p
const V = "1"
type Server struct{}
func (s *Server) Addr() string { return "" }
func main() {}
"#;
    let scm = make_extractor("go").extract("m.go", src).unwrap();
    let names: Vec<(&str, &str)> = scm
        .iter()
        .map(|s| (s.name.as_str(), s.kind.as_str()))
        .collect();
    assert!(names.contains(&("V", "constant")), "missing V: {names:?}");
    assert!(
        names.contains(&("Server", "class")),
        "missing Server: {names:?}"
    );
    assert!(
        names.contains(&("Addr", "method")),
        "missing Addr: {names:?}"
    );
    assert!(
        names.contains(&("main", "function")),
        "missing main: {names:?}"
    );
}

#[test]
fn scm_php_full_coverage() {
    let src = r#"<?php
namespace App;
class Ctrl { const M = 1; public function idx() {} }
function helper() {}
"#;
    let scm = make_extractor("php").extract("c.php", src).unwrap();
    let names: Vec<(&str, &str)> = scm
        .iter()
        .map(|s| (s.name.as_str(), s.kind.as_str()))
        .collect();
    assert!(
        names.contains(&("Ctrl", "class")),
        "missing Ctrl: {names:?}"
    );
    assert!(names.contains(&("M", "constant")), "missing M: {names:?}");
    assert!(
        names.contains(&("helper", "function")),
        "missing helper: {names:?}"
    );
}

#[test]
fn scm_javascript_full_coverage() {
    let src = r#"
class Svc { method() {} }
function top() {}
const handler = () => {};
const MAX = 42;
"#;
    let scm = make_extractor("javascript").extract("a.js", src).unwrap();
    let names: Vec<(&str, &str)> = scm
        .iter()
        .map(|s| (s.name.as_str(), s.kind.as_str()))
        .collect();
    assert!(names.contains(&("Svc", "class")), "missing Svc: {names:?}");
    assert!(
        names.contains(&("method", "method")),
        "missing method: {names:?}"
    );
    assert!(
        names.contains(&("top", "function")),
        "missing top: {names:?}"
    );
    assert!(
        names.contains(&("handler", "function")),
        "missing handler: {names:?}"
    );
    assert!(
        names.contains(&("MAX", "constant")),
        "missing MAX: {names:?}"
    );
}

#[test]
fn scm_rust_full_coverage() {
    let src = r#"
mod inner {}
struct Foo;
enum Bar { A }
trait Baz { fn required(&self); }
impl Foo { fn new() -> Self { Foo } }
fn free() {}
const C: i32 = 1;
"#;
    let scm = make_extractor("rust").extract("l.rs", src).unwrap();
    let names: Vec<(&str, &str)> = scm
        .iter()
        .map(|s| (s.name.as_str(), s.kind.as_str()))
        .collect();
    assert!(names.contains(&("Foo", "class")), "missing Foo: {names:?}");
    assert!(names.contains(&("Bar", "type")), "missing Bar: {names:?}");
    assert!(names.contains(&("Baz", "type")), "missing Baz: {names:?}");
    assert!(
        names.contains(&("new", "function")),
        "missing new: {names:?}"
    );
    assert!(
        names.contains(&("free", "function")),
        "missing free: {names:?}"
    );
    assert!(names.contains(&("C", "constant")), "missing C: {names:?}");
}

#[test]
fn scm_rust_generic_impl_methods_are_receiver_scoped() {
    // `impl<T> Wrapper<T>` and `impl<T> Trait for Wrapper<T>` used to lose
    // the receiver name because the tags.scm impl pattern only matched a
    // plain `type_identifier`, not a `generic_type`. Without the receiver
    // prefix, same-named methods from different impls in one file collide
    // in the stable symbol id and break round-trips.
    let src = r#"
struct Wrapper<T>(T);
impl<T> Wrapper<T> {
    fn into_inner(self) -> T { self.0 }
}
trait Take { fn take(self) -> i32; }
impl<T> Take for Wrapper<T> {
    fn take(self) -> i32 { 0 }
}
"#;
    let scm = make_extractor("rust").extract("l.rs", src).unwrap();
    let into_inner = scm
        .iter()
        .find(|s| s.name == "into_inner")
        .expect("into_inner should be extracted");
    assert_eq!(
        into_inner.qualified_name, "Wrapper.into_inner",
        "generic inherent impl method should carry receiver prefix"
    );
    let take = scm
        .iter()
        .find(|s| s.name == "take" && s.qualified_name.contains("Wrapper"))
        .expect("take should be extracted with Wrapper scope");
    assert_eq!(
        take.qualified_name, "Wrapper.take",
        "generic trait-for-impl method should carry receiver prefix"
    );
}

#[test]
fn scm_cpp_full_coverage() {
    let src = r#"
namespace ns { void helper() {} }
class Widget { void method(); };
void Widget::method() {}
"#;
    let scm = make_extractor("cpp").extract("w.cpp", src).unwrap();
    let names: Vec<(&str, &str)> = scm
        .iter()
        .map(|s| (s.name.as_str(), s.kind.as_str()))
        .collect();
    assert!(
        names.contains(&("helper", "function")),
        "missing helper: {names:?}"
    );
    assert!(
        names.contains(&("Widget", "class")),
        "missing Widget: {names:?}"
    );
    assert!(
        names.contains(&("Widget::method", "function")),
        "missing Widget::method: {names:?}"
    );
}

#[test]
fn scm_ruby_full_coverage() {
    let src = r#"
class Dog
  def bark; end
end
def top; end
"#;
    let scm = make_extractor("ruby").extract("d.rb", src).unwrap();
    let names: Vec<(&str, &str)> = scm
        .iter()
        .map(|s| (s.name.as_str(), s.kind.as_str()))
        .collect();
    assert!(names.contains(&("Dog", "class")), "missing Dog: {names:?}");
    assert!(
        names.contains(&("bark", "method")),
        "missing bark: {names:?}"
    );
}

#[test]
fn scm_perl_full_coverage() {
    let src = r#"
package Animal;
use strict;
use constant MAX_LEGS => 4;
use constant KINGDOM => 'Animalia';

sub new {
    my ($class, %args) = @_;
    return bless \%args, $class;
}

sub describe {
    my ($self) = @_;
    return "An animal";
}
1;
"#;
    let scm = make_extractor("perl").extract("Animal.pm", src).unwrap();
    let names: Vec<(&str, &str)> = scm
        .iter()
        .map(|s| (s.name.as_str(), s.kind.as_str()))
        .collect();
    assert!(
        names.contains(&("Animal", "class")),
        "missing Animal: {names:?}"
    );
    assert!(
        names.contains(&("new", "function")),
        "missing new: {names:?}"
    );
    assert!(
        names.contains(&("describe", "function")),
        "missing describe: {names:?}"
    );
    assert!(
        names.contains(&("MAX_LEGS", "constant")),
        "missing MAX_LEGS: {names:?}"
    );
    assert!(
        names.contains(&("KINGDOM", "constant")),
        "missing KINGDOM: {names:?}"
    );
}

#[test]
fn scm_elixir_full_coverage() {
    let src = r#"defmodule MyApp.Calculator do
  @moduledoc """
  A simple calculator module.
  """

  @type result :: {:ok, number()} | {:error, String.t()}

  @doc """
  Adds two numbers together.
  """
  def add(a, b) do
    a + b
  end

  @doc false
  defp validate(x) when is_number(x) do
    {:ok, x}
  end

  defmacro debug(expr) do
    quote do: IO.inspect(unquote(expr))
  end
end

defmodule MyApp.Types do
  @type name :: String.t()
  defguard is_positive(x) when is_number(x) and x > 0
end

defprotocol MyApp.Printable do
  @callback render(term()) :: String.t()
  def to_string(value)
end

defimpl MyApp.Printable, for: Integer do
  def to_string(value), do: Integer.to_string(value)
end
"#;
    let scm = make_extractor("elixir").extract("sample.ex", src).unwrap();

    let calc = scm
        .iter()
        .find(|s| s.name == "MyApp.Calculator")
        .expect("missing MyApp.Calculator");
    assert_eq!(calc.kind.as_str(), "class", "Calculator should be class");
    assert!(
        calc.summary.to_lowercase().contains("simple calculator"),
        "Calculator summary should mention 'simple calculator', got: {:?}",
        calc.summary
    );

    let add = scm.iter().find(|s| s.name == "add").expect("missing add");
    assert_eq!(add.kind.as_str(), "method", "add should be method");
    assert_eq!(add.qualified_name, "MyApp.Calculator.add");
    assert!(
        add.summary.contains("Adds two numbers"),
        "add summary should contain 'Adds two numbers', got: {:?}",
        add.summary
    );

    let validate = scm
        .iter()
        .find(|s| s.name == "validate")
        .expect("missing validate");
    assert_eq!(
        validate.kind.as_str(),
        "method",
        "validate should be method"
    );
    assert_eq!(validate.qualified_name, "MyApp.Calculator.validate");

    let debug = scm
        .iter()
        .find(|s| s.name == "debug")
        .expect("missing debug");
    assert_eq!(
        debug.kind.as_str(),
        "method",
        "debug macro should be method"
    );

    let result_type = scm
        .iter()
        .find(|s| s.name == "result")
        .expect("missing result type");
    assert_eq!(result_type.kind.as_str(), "type", "result should be type");
    assert_eq!(result_type.qualified_name, "MyApp.Calculator.result");

    let guard = scm
        .iter()
        .find(|s| s.name == "is_positive")
        .expect("missing is_positive");
    assert_eq!(guard.kind.as_str(), "method", "guard should be method");
    assert_eq!(guard.qualified_name, "MyApp.Types.is_positive");

    let name_type = scm
        .iter()
        .find(|s| s.name == "name")
        .expect("missing name type");
    assert_eq!(name_type.kind.as_str(), "type", "name should be type");

    let protocol = scm
        .iter()
        .find(|s| s.name == "MyApp.Printable")
        .expect("missing MyApp.Printable");
    assert_eq!(protocol.kind.as_str(), "type", "protocol should be type");

    let callback = scm
        .iter()
        .find(|s| s.name == "render")
        .expect("missing render callback");
    assert_eq!(callback.kind.as_str(), "type", "callback should be type");

    let _impl_sym = scm
        .iter()
        .find(|s| s.qualified_name.contains("Printable") && s.kind.as_str() == "class")
        .expect("missing Printable impl class");

    let to_str = scm
        .iter()
        .find(|s| s.name == "to_string")
        .expect("missing to_string");
    assert_eq!(to_str.kind.as_str(), "method", "to_string should be method");
}

#[test]
fn scm_erlang_full_coverage() {
    let src = r#"-module(mymod).
-define(MAX_ITER, 100).
-record(point, {x = 0, y = 0}).
-type number_pair() :: {number(), number()}.

add(X, Y) ->
    X + Y.

factorial(0) ->
    1;
factorial(N) when N > 0 ->
    N * factorial(N - 1).
"#;
    let scm = make_extractor("erlang").extract("mymod.erl", src).unwrap();
    let names: Vec<(&str, &str)> = scm
        .iter()
        .map(|s| (s.name.as_str(), s.kind.as_str()))
        .collect();
    assert!(
        names.contains(&("add", "function")),
        "missing add: {names:?}"
    );
    assert!(
        names.contains(&("factorial", "function")),
        "missing factorial: {names:?}"
    );
    assert!(
        names.contains(&("number_pair", "type")),
        "missing number_pair: {names:?}"
    );
    assert!(
        names.contains(&("point", "type")),
        "missing point: {names:?}"
    );
    assert!(
        names.contains(&("MAX_ITER", "constant")),
        "missing MAX_ITER: {names:?}"
    );
}

#[test]
fn scm_swift_full_coverage() {
    let src = r#"
class Svc { func run() {} }
func helper() -> Bool { true }
protocol P { func p() }
"#;
    let scm = make_extractor("swift").extract("s.swift", src).unwrap();
    let names: Vec<(&str, &str)> = scm
        .iter()
        .map(|s| (s.name.as_str(), s.kind.as_str()))
        .collect();
    assert!(names.contains(&("Svc", "class")), "missing Svc: {names:?}");
    assert!(
        names.contains(&("run", "function")),
        "missing run: {names:?}"
    );
    assert!(
        names.contains(&("helper", "function")),
        "missing helper: {names:?}"
    );
    assert!(names.contains(&("P", "type")), "missing P: {names:?}");
}

#[test]
fn scm_kotlin_full_coverage() {
    let src = r#"
fun greet(name: String): String {
    return "Hello, $name!"
}

class Calculator {
    fun add(a: Int, b: Int): Int {
        return a + b
    }
}

interface Clickable {
    fun click()
}

object AppConfig {
    val version = "1.0"
}

typealias StringList = List<String>

enum class Direction {
    NORTH, SOUTH, EAST, WEST
}

data class Point(val x: Int, val y: Int)
"#;
    let scm = make_extractor("kotlin").extract("main.kt", src).unwrap();
    let names: Vec<(&str, &str)> = scm
        .iter()
        .map(|s| (s.name.as_str(), s.kind.as_str()))
        .collect();
    assert!(
        names.contains(&("greet", "function")),
        "missing greet: {names:?}"
    );
    assert!(
        names.contains(&("Calculator", "class")),
        "missing Calculator: {names:?}"
    );
    assert!(names.contains(&("add", "method")), "missing add: {names:?}");
    assert!(
        names.contains(&("Clickable", "type")),
        "missing Clickable: {names:?}"
    );
    assert!(
        names.contains(&("AppConfig", "class")),
        "missing AppConfig: {names:?}"
    );
    assert!(
        names.contains(&("StringList", "type")),
        "missing StringList: {names:?}"
    );
    assert!(
        names.contains(&("Direction", "class")),
        "missing Direction: {names:?}"
    );
    assert!(
        names.contains(&("Point", "class")),
        "missing Point: {names:?}"
    );

    let method = scm.iter().find(|s| s.name == "add").unwrap();
    assert_eq!(method.kind, SymbolKind::Method);
    assert_eq!(method.qualified_name, "Calculator.add");
}

#[test]
fn scm_fortran_full_coverage() {
    let src = concat!(
        "! Computes the sum of two integers.\n",
        "function add(a, b) result(res)\n",
        "  integer, intent(in) :: a, b\n",
        "  integer :: res\n",
        "  res = a + b\n",
        "end function add\n",
        "\n",
        "! Greets the user.\n",
        "subroutine greet(name)\n",
        "  character(len=*), intent(in) :: name\n",
        "  print *, \"Hello, \" // name\n",
        "end subroutine greet\n",
        "\n",
        "module math_utils\n",
        "  implicit none\n",
        "  integer, parameter :: MAX_SIZE = 100\n",
        "  type :: Point\n",
        "    real :: x\n",
        "    real :: y\n",
        "  end type Point\n",
        "contains\n",
        "  ! Multiplies two reals.\n",
        "  function multiply(a, b) result(res)\n",
        "    real, intent(in) :: a, b\n",
        "    real :: res\n",
        "    res = a * b\n",
        "  end function multiply\n",
        "end module math_utils\n",
    );
    let scm = make_extractor("fortran").extract("math.f90", src).unwrap();
    let names: Vec<(&str, &str)> = scm
        .iter()
        .map(|s| (s.name.as_str(), s.kind.as_str()))
        .collect();

    assert!(
        names.contains(&("add", "function")),
        "missing add: {names:?}"
    );
    let add = scm.iter().find(|s| s.name == "add").unwrap();
    assert!(add.signature.contains("add"), "sig: {}", add.signature);
    assert!(
        add.summary.contains("Computes the sum"),
        "doc: {}",
        add.summary
    );
    assert_eq!(add.language, codesight_core::FORTRAN);

    assert!(
        names.contains(&("greet", "function")),
        "missing greet: {names:?}"
    );
    let greet = scm.iter().find(|s| s.name == "greet").unwrap();
    assert!(
        greet.signature.contains("subroutine"),
        "sig: {}",
        greet.signature
    );
    assert!(
        greet.summary.contains("Greets the user"),
        "doc: {}",
        greet.summary
    );

    assert!(
        names.contains(&("math_utils", "class")),
        "missing math_utils: {names:?}"
    );
    let module = scm.iter().find(|s| s.name == "math_utils").unwrap();
    assert!(
        module.signature.contains("module"),
        "sig: {}",
        module.signature
    );

    assert!(
        names.contains(&("multiply", "method")),
        "missing multiply: {names:?}"
    );
    let mul = scm.iter().find(|s| s.name == "multiply").unwrap();
    assert_eq!(mul.qualified_name, "math_utils.multiply");
    assert!(
        mul.summary.contains("Multiplies two reals"),
        "doc: {}",
        mul.summary
    );

    assert!(
        names.contains(&("Point", "type")),
        "missing Point: {names:?}"
    );
    let pt = scm.iter().find(|s| s.name == "Point").unwrap();
    assert!(pt.signature.contains("Point"), "sig: {}", pt.signature);

    assert!(
        names.contains(&("MAX_SIZE", "constant")),
        "missing MAX_SIZE: {names:?}"
    );
    let cst = scm.iter().find(|s| s.name == "MAX_SIZE").unwrap();
    assert_eq!(cst.qualified_name, "math_utils.MAX_SIZE");

    let source_len = src.len();
    for sym in &scm {
        assert!(
            sym.end_byte <= source_len,
            "byte offsets out of range for {}",
            sym.name
        );
    }
}

#[test]
fn scm_gleam_full_coverage() {
    let src = r#"pub type Color {
  Red
  Green
  Blue
}

pub type Alias = String

pub const max_size: Int = 100

// Greet a user
pub fn greet(name: String) -> String {
  "Hello, " <> name
}

fn helper(x: Int) -> Int {
  x + 1
}
"#;
    let scm = make_extractor("gleam").extract("app.gleam", src).unwrap();
    let names: Vec<(&str, &str)> = scm
        .iter()
        .map(|s| (s.name.as_str(), s.kind.as_str()))
        .collect();

    assert!(
        names.contains(&("Color", "type")),
        "missing Color: {names:?}"
    );
    let color = scm.iter().find(|s| s.name == "Color").unwrap();
    assert!(
        color.signature.contains("Color"),
        "Color signature missing name: {:?}",
        color.signature
    );

    assert!(
        names.contains(&("Alias", "type")),
        "missing Alias: {names:?}"
    );

    assert!(
        names.contains(&("max_size", "constant")),
        "missing max_size: {names:?}"
    );

    assert!(
        names.contains(&("greet", "function")),
        "missing greet: {names:?}"
    );
    let greet = scm.iter().find(|s| s.name == "greet").unwrap();
    assert!(
        greet.signature.contains("String"),
        "greet signature missing String: {:?}",
        greet.signature
    );
    assert!(
        greet.summary.contains("Greet a user"),
        "greet summary missing 'Greet a user': {:?}",
        greet.summary,
    );

    assert!(
        names.contains(&("helper", "function")),
        "missing helper: {names:?}"
    );
}
