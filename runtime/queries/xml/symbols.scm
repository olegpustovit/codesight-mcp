; language: xml
; extensions: xml svg xsl xslt xsd plist pom csproj fsproj vcxproj wixproj props targets resx xaml
; grammar: xml

; ── Top-level element (root) as type ────────────────────────
; The root element acts as a namespace/type container.
(document
  (element
    (STag (Name) @name)) @definition.type)

; ── Nested elements as constants ────────────────────────────
; Child elements inside content represent data fields / constants.
(element
  (content
    (element
      (STag (Name) @name)) @definition.constant))

; ── Empty elements as constants ─────────────────────────────
(element
  (content
    (element
      (EmptyElemTag (Name) @name)) @definition.constant))

; ── Script src references as functions ──────────────────────
; <script src="..."> style references.
(element
  (STag
    (Name) @_tag
    (Attribute
      (Name) @_attr
      (AttValue) @name))
  (#eq? @_tag "script")
  (#eq? @_attr "src")) @definition.function
