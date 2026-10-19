# Übersetzungen

Die Oberfläche ist mehrsprachig; **Deutsch ist die Ausgangssprache**, dazu kommt Englisch. Welche Sprache ein Benutzer sieht, steht in
`docs/benutzer-und-login.md` (Abschnitt „Sprache“).

## Aufbau

- Texte stehen in **Fluent-Katalogen** (`locales/<code>/*.ftl`, [Syntax](https://projectfluent.org/fluent/guide/)), je Sprache dieselben Dateien.
  Sie werden beim Bauen ins Binary eingebettet (`rust-embed`); zur Laufzeit wird nichts nachgeladen.
- Code: `src/i18n.rs`. `Lang` ist eine Sprache, `t.get("id")`, `t.get1("id", "name", wert)`, `t.get2(…)` und `t.fmt("id", &[…])` liefern Texte.
  Jede Seite hat das Template-Feld `t` (Sprache des Benutzers, auf der Anmeldeseite die des Browsers).
- Meldungen aus der Fachlogik (Prüfungen, Konflikte) sind `Msg`-Werte (`Msg::new("id").with("name", wert)`) und werden erst dort übersetzt, wo sie
  angezeigt werden; `Error::Invalid`/`Error::Conflict` tragen ein `Msg`. Ein `Msg::Text` wird unverändert gezeigt (z. B. Fehlertexte eines Geräts).
- Fehlerseiten ohne Formular (z. B. „Nicht gefunden“) nehmen die Sprache der Anfrage (`i18n::current()`, gesetzt in `web::request_language` und
  nach der Anmeldung im `CurrentUser`-Extractor).
- Zahlen in deutschen Texten bekommen ein Dezimalkomma (Formatter der deutschen Fluent-Bundles).
- Messwerte, Einheiten und Schwellwertfelder formatiert `sensors::format` je Sprache (Dezimalkomma bzw. -punkt, „Std.“/„h“).
- Gespeicherte Meldungen (Sensorzustand, Ereignislog, Sendelog der E-Mails) stehen als kodiertes `Msg` in der Datenbank (`Msg::encode`/`decode`)
  und erscheinen so in der Sprache des Betrachters. Ältere Einträge aus der Zeit vor der Umstellung bleiben deutscher Klartext.
- Kataloge je Bereich: `common` (Rahmen, Status, gemeinsame Wörter), `users`, `devices` (Gruppen, Geräte, Übersicht), `sensors` (Sensoren,
  Discovery, Detailseite), `values` (Werte suchen, bekannte Tabellen), `mibs`, `templates`, `credentials`, `mail`, `order`, `js`.

## Texte in JavaScript

- Die Skripte in `static/` bekommen ihre Texte aus einem JSON-Datenblock im Seitenrahmen
  (`<script type="application/json" id="i18n">`, erzeugt von `Lang::js_texts()`); die CSP lässt ihn zu, weil er nicht ausgeführt wird.
- Er enthält alle Nachrichten aus `js.ftl` (IDs `js-…`, im Skript ohne Vorsilbe): `T('samples-sum')` bzw. `window.opennetmonText(…)` aus
  `static/theme.js`. Platzhalter heißen `%1`, `%2` (`T('samples-from-many', n)`); Fluent-Variablen sind in `js.ftl` nicht erlaubt (Test).
- Datums- und Zahlenformat kommen ebenfalls aus dem Katalog: `js-decimal` (`,`/`.`), `js-date` (`DD.MM.YYYY`/`MM/DD/YYYY`), `js-day`.
- Die Tests suchen `T('…')` in `static/*.js`: jede verwendete ID muss existieren, jede `js-`-Nachricht verwendet werden.

## Regeln

- Nachrichten-IDs in Kleinbuchstaben mit Bindestrich, nach Bereich vorangestellt (`nav-…`, `login-…`, `users-…`, `flash-…`).
- Variablen (`{ $name }`) und Mehrzahl über Selektoren:

  ```ftl
  login-wait-minutes =
      { $minutes ->
          [one] { $minutes } Minute
         *[other] { $minutes } Minuten
      }
  ```

- Platzhalter-Werte werden beim Einsetzen in HTML von askama maskiert; Kataloge enthalten **kein HTML**.
- Die Tests in `src/i18n.rs` prüfen: alle Kataloge sind fehlerfrei, jede Sprache hat dieselben Dateien, Nachrichten und Variablen wie Deutsch,
  jede im Code oder in Templates verwendete ID existiert und jede Nachricht wird verwendet.

## Neue Sprache

1. `Lang` in `src/i18n.rs` um die Sprache erweitern (`ALL`, `code`, `native_name`).
2. `locales/de/` nach `locales/<code>/` kopieren und übersetzen; Mehrzahlformen nach den Regeln der Sprache (CLDR, z. B. `one`, `few`, `many`, `other`).
3. `cargo test` – die Katalogtests zeigen fehlende oder überzählige Nachrichten.

Nicht übersetzt werden gespeicherte Namen (Geräte, Gruppen, Sensoren, Vorlagen), Werte von Geräten und die Projektdokumentation in `docs/`.
