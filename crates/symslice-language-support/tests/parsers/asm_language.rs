//! Integration tests: asm language.

use std::path::Path;
use symslice_core::{StoredSymbol, SymbolKind};
use symslice_language_support::LanguageRegistry;
use symslice_language_support::languages::asm::extractor::AsmExtractor;

// ---------------------------------------------------------------------------
// Inline fixtures (matching the reference Python test module)
// ---------------------------------------------------------------------------

const WLA_DX_SOURCE: &str = "\
; Boot and initialization
.include \"src/config/config.inc\"

.define maxNumberOopObjs 48
.define oopStackTst $aa55
.def oopStackObj.length _sizeof_oopStackObj

.ENUM 0 export
OBJR_noErr db
OBJR_kill db
.ende

.STRUCT oopStackObj
flags       db
id          db
num         dw
properties  dw
dp          dw
init        dw
play        dw
kill        dw
.ENDST

.ramsection \"global vars\" bank 0 slot 2
  STACK_strt dw
  STACK_end dw
.ends

.section \"oophandler\"

;clear oop stack
core.object.init:
    php
    phd
    rep #$31
    lda #ZP
    tcd
    lda #0
    ldy #OopStackEnd-OopStack
    ldx #OopStack
    jsr ClearWRAM
    pld
    plp
    rts

;in:y=number of object to create, a:call parameter x:pointer
core.object.create:
    php
    phd
    rep #$31
    pha
    tdc
    pea ZP
    pld
    rts

.ends

.section \"ScummVM dispatch\" superfree

scummvm.executeOpcode:
    rep #$31
    jsr _fetchByte
    sta SCUMM.currentOpcode
    rts

_fetchByte:
    sep #$20
    lda [SCUMM.scriptPtr]
    rep #$20
    and #$00FF
    rts

.ends

.macro CLASS
.redefine __classid \\1
T_CLSS_\\@:
.db \"\\1\", 0
.endm

.macro METHOD
.redefine __method \\1
_\\1:
.endm

.macro SCRIPT
\\1:
.dw $BADE
.endm
";

const NASM_STYLE_SOURCE: &str = "\
; Simple x86 NASM example
section .text
    global _start

_start:
    mov eax, 1
    mov ebx, 0
    int 0x80

print_hello:
    push ebp
    mov ebp, esp
    ret

%define BUFFER_SIZE 1024
%macro PROLOGUE 0
    push ebp
    mov ebp, esp
%endmacro

section .data
    msg db \"Hello\", 0
    len equ $ - msg

section .bss
    buffer resb 1024
";

const GAS_STYLE_SOURCE: &str = "\
    .text
    .globl main

main:
    pushq %rbp
    movq %rsp, %rbp
    ret

.type helper, @function
helper:
    ret

.set MAX_COUNT, 256
.equ STACK_SIZE, 4096

.macro save_regs
    pushq %rbx
    pushq %r12
.endm

.data
message:
    .asciz \"Hello\"

.bss
    .lcomm buffer, 1024
";

const CA65_STYLE_SOURCE: &str = "\
.segment \"CODE\"

.proc main
    lda #$00
    sta $2000
    rts
.endproc

.proc irq_handler
    pha
    pla
    rti
.endproc

.define SCREEN_WIDTH 32
PPU_CTRL = $2000

.macro set_ppu_addr addr
    lda #.hibyte(addr)
    sta $2006
    lda #.lobyte(addr)
    sta $2006
.endmacro
";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn extract_wla_dx() -> Vec<StoredSymbol> {
    AsmExtractor
        .extract("core/oop.65816", WLA_DX_SOURCE)
        .expect("WLA-DX extraction should succeed")
}

fn extract_nasm() -> Vec<StoredSymbol> {
    AsmExtractor
        .extract("hello.asm", NASM_STYLE_SOURCE)
        .expect("NASM extraction should succeed")
}

fn extract_gas() -> Vec<StoredSymbol> {
    AsmExtractor
        .extract("main.s", GAS_STYLE_SOURCE)
        .expect("GAS extraction should succeed")
}

fn extract_ca65() -> Vec<StoredSymbol> {
    AsmExtractor
        .extract("main.s", CA65_STYLE_SOURCE)
        .expect("CA65 extraction should succeed")
}

fn names_of_kind(symbols: &[StoredSymbol], kind: SymbolKind) -> Vec<String> {
    symbols
        .iter()
        .filter(|s| s.kind == kind)
        .map(|s| s.name.clone())
        .collect()
}

fn find_by_name<'a>(symbols: &'a [StoredSymbol], name: &str) -> Option<&'a StoredSymbol> {
    symbols.iter().find(|s| s.name == name)
}

// ===========================================================================
// WLA-DX dialect (tests 1–11)
// ===========================================================================
/// Global labels are extracted as functions.
#[test]
fn test_parse_wla_dx_labels() {
    let symbols = extract_wla_dx();
    let funcs = names_of_kind(&symbols, SymbolKind::Function);
    assert!(
        funcs.contains(&"core.object.init".to_string()),
        "missing core.object.init"
    );
    assert!(
        funcs.contains(&"core.object.create".to_string()),
        "missing core.object.create"
    );
    assert!(
        funcs.contains(&"scummvm.executeOpcode".to_string()),
        "missing scummvm.executeOpcode"
    );
}
/// _prefixed local labels are excluded.
#[test]
fn test_parse_wla_dx_local_labels_excluded() {
    let symbols = extract_wla_dx();
    let all_names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(
        !all_names.contains(&"_fetchByte"),
        "_fetchByte (local label) should be excluded"
    );
}
/// .section directives are extracted as class symbols.
#[test]
fn test_parse_wla_dx_sections() {
    let symbols = extract_wla_dx();
    let classes = names_of_kind(&symbols, SymbolKind::Class);
    assert!(
        classes.contains(&"oophandler".to_string()),
        "missing oophandler section"
    );
    assert!(
        classes.contains(&"ScummVM dispatch".to_string()),
        "missing 'ScummVM dispatch' section"
    );
}
/// .macro definitions are extracted as function symbols.
#[test]
fn test_parse_wla_dx_macros() {
    let symbols = extract_wla_dx();
    let funcs = names_of_kind(&symbols, SymbolKind::Function);
    assert!(funcs.contains(&"CLASS".to_string()), "missing CLASS macro");
    assert!(
        funcs.contains(&"METHOD".to_string()),
        "missing METHOD macro"
    );
    assert!(
        funcs.contains(&"SCRIPT".to_string()),
        "missing SCRIPT macro"
    );
}
/// .define/.def are extracted as constant symbols.
/// The .def directive with dotted name (oopStackObj.length) requires regex to
/// accept '.' in names — current impl uses \w+ which excludes dots.
#[test]
fn test_parse_wla_dx_constants() {
    let symbols = extract_wla_dx();
    let consts = names_of_kind(&symbols, SymbolKind::Constant);
    assert!(
        consts.contains(&"maxNumberOopObjs".to_string()),
        "missing maxNumberOopObjs"
    );
    assert!(
        consts.contains(&"oopStackTst".to_string()),
        "missing oopStackTst"
    );
    assert!(
        consts.contains(&"oopStackObj.length".to_string()),
        "missing oopStackObj.length — .def with dotted name not captured"
    );
}
/// .struct definitions are extracted as type symbols.
#[test]
fn test_parse_wla_dx_structs() {
    let symbols = extract_wla_dx();
    let types = names_of_kind(&symbols, SymbolKind::Type);
    assert!(
        types.contains(&"oopStackObj".to_string()),
        "missing oopStackObj struct"
    );
}
/// Named .enum members are extracted as constants.
#[test]
fn test_parse_wla_dx_enums() {
    let symbols = extract_wla_dx();
    let consts = names_of_kind(&symbols, SymbolKind::Constant);
    assert!(
        consts.contains(&"OBJR_noErr".to_string()),
        "missing OBJR_noErr enum member"
    );
    assert!(
        consts.contains(&"OBJR_kill".to_string()),
        "missing OBJR_kill enum member"
    );
}
/// .ramsection directives are extracted as class symbols.
#[test]
fn test_parse_wla_dx_ramsections() {
    let symbols = extract_wla_dx();
    let classes = names_of_kind(&symbols, SymbolKind::Class);
    assert!(
        classes.contains(&"global vars".to_string()),
        "missing 'global vars' ramsection"
    );
}
/// Preceding ;comments are captured as docstrings (stored in `summary`).
#[test]
fn test_parse_wla_dx_docstrings() {
    let symbols = extract_wla_dx();

    let init = find_by_name(&symbols, "core.object.init").expect("core.object.init should exist");
    assert!(
        init.summary.contains("clear oop stack"),
        "core.object.init summary should contain 'clear oop stack', got: {}",
        init.summary
    );

    let create =
        find_by_name(&symbols, "core.object.create").expect("core.object.create should exist");
    assert!(
        create.summary.contains("number of object to create"),
        "core.object.create summary should contain 'number of object to create', got: {}",
        create.summary
    );
}
/// Labels inside sections get qualified names (section::label).
#[test]
fn test_parse_wla_dx_qualified_names() {
    let symbols = extract_wla_dx();

    let init = find_by_name(&symbols, "core.object.init").expect("core.object.init should exist");
    assert_eq!(
        init.qualified_name, "oophandler::core.object.init",
        "qualified_name should be section-qualified"
    );
}
/// All symbols should have language='asm'.
#[test]
fn test_parse_wla_dx_language_field() {
    let symbols = extract_wla_dx();
    assert!(!symbols.is_empty(), "should extract at least one symbol");
    for s in &symbols {
        assert_eq!(
            s.language.as_str(),
            "asm",
            "symbol {} has wrong language",
            s.name
        );
    }
}

// ===========================================================================
// NASM dialect (tests 12–16)
// ===========================================================================
/// NASM-style label extraction. _start excluded due to _ prefix convention.
#[test]
fn test_parse_nasm_labels() {
    let symbols = extract_nasm();
    let funcs = names_of_kind(&symbols, SymbolKind::Function);
    assert!(
        funcs.contains(&"print_hello".to_string()),
        "missing print_hello label"
    );
}
/// NASM %define extraction.
#[test]
fn test_parse_nasm_defines() {
    let symbols = extract_nasm();
    let consts = names_of_kind(&symbols, SymbolKind::Constant);
    assert!(
        consts.contains(&"BUFFER_SIZE".to_string()),
        "missing BUFFER_SIZE"
    );
}
/// NASM %macro extraction.
#[test]
fn test_parse_nasm_macros() {
    let symbols = extract_nasm();
    let funcs = names_of_kind(&symbols, SymbolKind::Function);
    assert!(
        funcs.contains(&"PROLOGUE".to_string()),
        "missing PROLOGUE macro"
    );
}
/// NASM equ constant extraction.
#[test]
fn test_parse_nasm_equ() {
    let symbols = extract_nasm();
    let consts = names_of_kind(&symbols, SymbolKind::Constant);
    assert!(
        consts.contains(&"len".to_string()),
        "missing len equ constant"
    );
}
/// NASM section extraction — sections should appear as class symbols named ".text", ".data", ".bss".
#[test]
fn test_parse_nasm_sections() {
    let symbols = extract_nasm();
    let classes = names_of_kind(&symbols, SymbolKind::Class);
    assert!(
        classes.contains(&".text".to_string()),
        "missing .text section"
    );
    assert!(
        classes.contains(&".data".to_string()),
        "missing .data section"
    );
    assert!(
        classes.contains(&".bss".to_string()),
        "missing .bss section"
    );
}

// ===========================================================================
// GAS dialect (tests 17–19)
// ===========================================================================
/// GAS-style label extraction.
#[test]
fn test_parse_gas_labels() {
    let symbols = extract_gas();
    let funcs = names_of_kind(&symbols, SymbolKind::Function);
    assert!(funcs.contains(&"main".to_string()), "missing main label");
    assert!(
        funcs.contains(&"helper".to_string()),
        "missing helper label"
    );
}
/// GAS .set/.equ constant extraction.
#[test]
fn test_parse_gas_constants() {
    let symbols = extract_gas();
    let consts = names_of_kind(&symbols, SymbolKind::Constant);
    assert!(
        consts.contains(&"MAX_COUNT".to_string()),
        "missing MAX_COUNT"
    );
    assert!(
        consts.contains(&"STACK_SIZE".to_string()),
        "missing STACK_SIZE"
    );
}
/// GAS .macro extraction.
#[test]
fn test_parse_gas_macros() {
    let symbols = extract_gas();
    let funcs = names_of_kind(&symbols, SymbolKind::Function);
    assert!(
        funcs.contains(&"save_regs".to_string()),
        "missing save_regs macro"
    );
}

// ===========================================================================
// CA65 dialect (tests 20–23)
// ===========================================================================
/// CA65 .proc extraction.
#[test]
fn test_parse_ca65_procs() {
    let symbols = extract_ca65();
    let funcs = names_of_kind(&symbols, SymbolKind::Function);
    assert!(funcs.contains(&"main".to_string()), "missing main proc");
    assert!(
        funcs.contains(&"irq_handler".to_string()),
        "missing irq_handler proc"
    );
}
/// CA65 .segment extraction.
#[test]
fn test_parse_ca65_segments() {
    let symbols = extract_ca65();
    let classes = names_of_kind(&symbols, SymbolKind::Class);
    assert!(
        classes.contains(&"CODE".to_string()),
        "missing CODE segment"
    );
}
/// CA65 constant extraction (.define and NAME = VALUE).
#[test]
fn test_parse_ca65_constants() {
    let symbols = extract_ca65();
    let consts = names_of_kind(&symbols, SymbolKind::Constant);
    assert!(
        consts.contains(&"SCREEN_WIDTH".to_string()),
        "missing SCREEN_WIDTH from .define"
    );
    assert!(
        consts.contains(&"PPU_CTRL".to_string()),
        "missing PPU_CTRL from = assignment"
    );
}
/// CA65 .macro/.endmacro extraction.
#[test]
fn test_parse_ca65_macros() {
    let symbols = extract_ca65();
    let funcs = names_of_kind(&symbols, SymbolKind::Function);
    assert!(
        funcs.contains(&"set_ppu_addr".to_string()),
        "missing set_ppu_addr macro"
    );
}

// ===========================================================================
// Edge cases (tests 24–25)
// ===========================================================================
/// Empty file should return no symbols.
#[test]
fn test_parse_empty_file() {
    let symbols = AsmExtractor
        .extract("empty.asm", "")
        .expect("empty file extraction should succeed");
    assert!(symbols.is_empty(), "empty file should yield no symbols");
}
/// File with only comments should return no symbols.
#[test]
fn test_parse_comments_only() {
    let source = "; This is a comment\n; Another comment\n";
    let symbols = AsmExtractor
        .extract("comments.asm", source)
        .expect("comments-only extraction should succeed");
    assert!(
        symbols.is_empty(),
        "comments-only file should yield no symbols"
    );
}

// ===========================================================================
// Registry / extension mapping (tests 26–27)
// ===========================================================================
/// Assembly file extensions are properly mapped to the asm extractor.
#[test]
fn test_extension_mapping() {
    let registry = LanguageRegistry::default();

    for ext in &["asm", "s", "65816", "inc", "z80", "spc"] {
        let path_str = format!("test.{ext}");
        let path = Path::new(&path_str);
        let extractor = registry.extractor_for_path(path);
        assert!(
            extractor.is_some(),
            "extension .{ext} should map to an extractor"
        );
        assert_eq!(
            extractor.unwrap().id().as_str(),
            "asm",
            "extension .{ext} should map to the asm extractor"
        );
    }
}
/// asm is registered in LanguageRegistry and discoverable.
#[test]
fn test_language_in_registry() {
    let registry = LanguageRegistry::default();
    // Verify asm is reachable by looking up a canonical asm extension.
    let extractor = registry.extractor_for_path(Path::new("test.asm"));
    assert!(
        extractor.is_some(),
        "asm should be registered in LanguageRegistry"
    );
    assert_eq!(extractor.unwrap().id().as_str(), "asm");
}
