# Ranking puts apps and commands in one list

This page explains how lyncast finds the desktop entries, matches a query against them and the built-in commands, and orders the rows. The [decision log](../decisions.md#app-index-and-ranking) wins any disagreement. TinyCast's launcher doc is the behaviour spec ([notes/06](../research/notes/06-tinycast-behaviour-spec.md#2-launcher)), and lyncast ports none of its code. The scorer measurements are in [notes/10](../research/notes/10-frizbee-tinycast-scoring.md).

Every fixture on this page is a fenced block that a test reads, so this page is the only copy of them. [The fixture formats](#the-fixture-formats-are-strict) are at the end.

## One keystroke is one frizbee pass and one sort

`lyncast-core` holds an `Index` of every searchable entry, the apps from the scan and the nine built-in commands. `Index::rows(text, &Visits, now)` returns the rows for one query. A typed query fills Results. A query that is empty or only whitespace once folded fills Suggestions, then Commands. The fold drops format characters and marks with a nonzero canonical combining class, so a lone soft hyphen is the empty query.

A typed query takes these steps:

1. Fold the query once ([below](#one-fold-runs-on-every-field-and-every-query)). Remove its separators to get frizbee's needle.
2. Run frizbee once over one flat table of haystacks. Each haystack is one field of one entry, folded when the index was built, and the table keeps each entry's fields together.
3. Gather each entry's matches. An entry shows when any of its fields is exact or passes the Medium threshold.
4. Give each shown entry one sort key that holds the comparator's rules in order, and sort the keys.
5. Build a `Row` for each key, with its subject, its section, and its title.

The binary does the IO. It walks the XDG directories, reads the files and the environment, looks up `TryExec` and the Exec program, and prints the rows. Everything a file says is decided by the pure parser in `lyncast-core`.

## frizbee scores with TinyCast's points

TinyCast's scorer gives 4 points when the query's first character lands on the text's first, 3 on a word start, 2 anywhere else, and takes 1 for each gap. lyncast gives frizbee 0.13.0 those points as its weights:

| frizbee weight | Value | TinyCast rule it makes |
|---|---|---|
| `match_score` | 2 | any other character, 2 |
| `prefix_bonus` | 2 | first on first, 4 |
| `delimiter_bonus`, `capitalization_bonus` | 1 | word start, 3, after a separator or at a capital after a lowercase letter |
| `gap_open_penalty`, `gap_extend_penalty` | 1, 0 | each gap, −1, whatever its length |
| `mismatch_penalty` | 4 | never worth a substitution |
| `matching_case_bonus`, `exact_match_bonus` | 0 | none |

`max_typos` is 0, so every query character must align. frizbee's own weights (12 per character, gaps of 5 plus 1 per character) fit TinyCast's threshold badly. The best linear threshold on them agreed on 75% of shown pairs and inverted 2,239 orders (measured, notes/10).

With these weights, TinyCast's Medium sensitivity applies as written. A field shows when its score is at least 1.5·(L−2)+4, where L is the query's length without separators. The code checks `2·score ≥ 3·L + 2`, so it needs no floats. The query is trimmed, and so is each field's lowercased fold, because GLib keeps a value's trailing whitespace. A field whose trimmed lowercased fold equals the folded query is exact. An exact field always shows and beats every score. frizbee reports `exact` only for case-sensitive byte equality, so lyncast decides exactness itself.

frizbee scores a match window of up to 1,024 bytes by a full alignment, and a longer one by a greedy match that can miss the best. The window lies inside one field, so frizbee scores each folded field's first 1,024 bytes, cut at a character boundary, in the index and in `score`. Exactness and prefixes still compare the whole field, so a cut field is exact only when all of it equals the query. `a_field_cut_at_1024_bytes_is_not_exact` in `tests/ranking.rs` pins this with generated text, because the doc's fixture lines cannot hold 1 KB. A query of 1,024 `x`s against a Name of 1,500 `x`s scores 2,050, 4 for the first `x` and 2 for each of the other 1,023, and is not exact, so the Name does not take rule 3's standing.

With lyncast's weights, frizbee 0.13.0 can still score below the best alignment when a gap is long. There is no fixed cutoff. A scan of gaps of 1 to 1,000 characters found the first drop at gaps of 94 to 127 bytes, depending on the needle, and a longer gap can score the best again. `ab` on `a`, 126 `z`s, and `b` scores 5, the best. With 127 `z`s it scores 3, with 128 or 200 it scores 5 again, and with 385 or 1,000 it scores 3. `ab` on `Audio `, 400 `z`s, and ` Bridge` scores 3, below Medium's 4, so that app does not show, although its best alignment scores 6. `abc` and `abcd` first drop with 95 `z`s in each gap, and `те` with 63 `ж`, 126 bytes. `тел` on `т`, 46 `ж`, `е`, 46 `ж`, and `л` scores 6, and with 47 `ж`, 94 bytes, in each gap it scores 5. frizbee's own weights score `ab` 23 at each of those lengths, so the zero gap-extend penalty looks like the trigger (measured, 2026-10-16). The bench's longest field holds 52 characters. A needle with more characters than the longest field cannot align, because each query character needs its own field character. `Index::rows` then returns no rows before frizbee builds its matcher, whose setup grows with the needle.

`Index::rows` gives no rows for query text over 1,024 bytes, before the fold, so a pasted page costs nothing. frizbee sees at most a field's first 1,024 bytes, so such a query could align only if the fold or the needle made it shorter, and it shows nothing then too. Full-width text is the exception the cap makes. A full-width letter is 3 bytes and folds to a 1-byte one, so 342 full-width letters, 1,026 bytes, show nothing, although a field may hold their 342-byte fold. Accented letters, combining marks, and separators make text shorter too. Text that would fold to the empty query, such as 1,025 spaces, also gives no rows. `a_query_over_1024_bytes_shows_nothing` in `tests/ranking.rs` pins the cap and the full-width case, because the doc's fixture lines cannot hold 1 KB.

The query's separators (space, tab, newline, and `- . / ( ) [ ]`) are removed before frizbee sees it. Against a reference scorer written from TinyCast's doc, removing them agreed on 305 of 305 multiword pairs, and keeping them agreed on 301 (measured). The field keeps its separators and its case, because frizbee needs both for word starts. lyncast sets frizbee to ignore case. Its default matches case-sensitively once the needle holds a capital, and a folded needle can still hold one with no lowercase form, such as `ℝ`.

The corpus is 225 names in `bench/score-map`, 216 app names and then the nine command titles, with 3,451 queries. frizbee gave the reference scorer's points on 98.4% of 17,596 aligned pairs and was 1 point off on the rest. It agreed with the reference on 9,861 of 9,960 shown pairs and inverted no order among 14,375 pairs that the reference scored apart (measured, notes/10). Three causes explain the 1-point differences:

- frizbee knows no word start at a letter after a digit (`1Password`) or at the last capital of a run before a lowercase letter (`BBEdit`, `KCalc`), so those score 1 lower.
- frizbee's local alignment can let a later query character take the first-on-first 4 alone. `ac` on Calculator scores 4 through its `c`, where the reference scorer gives 3.
- frizbee counts every ASCII character that is not a letter or digit as a separator. TinyCast's set is the ten above. So `_`, `+`, `:`, `&`, and `!` start words in lyncast and not in TinyCast.

These lines pin frizbee's real scores. `N shows` passes Medium, `N below` aligns and does not pass, `exact` is a folded equality, and `∅` does not align. The first block is TinyCast's own examples:

```score
code | Xcode => 8 shows
pec | Accessibility Inspector => 6 shows
pec | Previous Track => 6 shows
vsc | Visual Studio Code => 8 shows
chrome | Google Chrome => 13 shows
stack | OrbStack => 11 shows
ap | App Store => 6 shows
ap | AirPort Utility => 6 shows
edit | BBEdit => 8 shows
pass | 1Password => 8 shows
```

TinyCast gives `edit` on BBEdit and `pass` on 1Password 9. The rest match TinyCast's points. In the next block, the reference scorer gives `ac` on Calculator 3, which is below Medium.

```score
ac | Calculator => 4 shows
firef | Firefox => 12 shows
i | Firefox => 2 below
fx | Firefox => 5 shows
Firefox | firefox => exact
ｆｉｒｅ | Firefox => 10 shows
visual studio | Visual Studio Code => 26 shows
visualstudio | Visual Studio Code => 26 shows
тел | Телеграм => 8 shows
groß | GROẞ => exact
файлы | Файлы => exact
οδος | ΟΔΟΣ => exact
ℝ studio | ℝ Studio => exact
firefox | "Firefox " => exact
files | Файлы => ∅
- | Firefox => ∅
f-i.r/e(f)o[x] | Firefox => 16 shows
coe | Xcode => 5 below
s | App Store => 3 shows
```

`visual studio` and `visualstudio` score the same because the query loses its separators. `-` is a typed query with nothing left to match. `f-i.r/e(f)o[x]` holds the seven separators that are not whitespace and loses them all, and L counts the 7 characters left, so 16 passes Medium's 11.5. `coe` on Xcode scores 5, below the 5.5 that L 3 needs, and a word start alone, `s` on App Store's `Store`, scores 3 and passes L 1's 2.5.

## One fold runs on every field and every query

TinyCast folds every text the same way, once per field when the index is built and once per query. lyncast's fold takes these steps:

1. Narrow the full-width forms U+FF01 to U+FF5E to ASCII and U+3000 to a space. This is the calculator's fold, moved to `fold.rs`, and the calculator still runs only this step.
2. Drop format characters (general category Cf), such as the soft hyphen and zero-width spaces. `unicode-normalization` has no category data, so `fold.rs` keeps the Cf ranges as a table.
3. Decompose (NFD), drop every character with a nonzero canonical combining class, and recompose (NFC). This removes accents and other diacritics.
4. Lowercase every non-ASCII character and keep the ASCII capitals. Final sigma `ς` becomes `σ`, as Unicode case folding has it, so `ΟΔΟΣ` meets `οδος`. frizbee scores this form. Its word starts read only ASCII capitals, and it cannot pair a non-ASCII capital with a lowercase of another UTF-8 length, so `groß` would never find `GROẞ`. Lowercase the ASCII too for exactness, prefixes, and past search terms.

ASCII text skips steps 1 to 3. The fold does not fold half-width katakana, the full-width signs U+FFE0 to U+FFE6 (`￥`), compatibility ligatures (`ﬁ`), or `ß` to `ss`.

Each line reads `input => frizbee's form | lowercased`. Either side may write an invisible character as `\u{AD}`, and a no-break space is not a format character, so it stays.

```fold
Ｆｉｒｅｆｏｘ => Firefox | firefox
Ångström => Angstrom | angstrom
Soft\u{AD}ware => Software | software
İstanbul => Istanbul | istanbul
Café => Cafe | cafe
が => か | か
ｶﾞ => ｶﾞ | ｶﾞ
￥ => ￥ | ￥
Straße => Straße | straße
GROẞ => GROß | groß
Телеграм => телеграм | телеграм
ΟΔΟΣ => οδοσ | οδοσ
οδος => οδοσ | οδοσ
no\u{A0}break => no\u{A0}break | no\u{A0}break
```

## Fields decide whether an entry shows and how it ranks

An app has four kinds of field. A command has a title and keywords.

| Field | An app's value | A command's value | Ranks |
|---|---|---|---|
| Title | `Name[locale]` | its title, such as Clipboard History | yes |
| Alternate title | the unlocalized `Name`, when it folds differently | none | yes |
| Subtitle | `GenericName[locale]` | none | yes, and an exact subtitle is rule 5 |
| Keyword | each of `Keywords[locale]` and `Keywords`, and with a subtitle, the title and subtitle joined both ways with a space | a few each, in the table below | no, it only makes the entry show |

Exec, Icon, and the desktop-file ID are not matched, as TinyCast does not match bundle IDs.

| Command | Keywords |
|---|---|
| Clipboard History | paste |
| Open Config | settings, preferences |
| Reload Apps | refresh |
| Quit lyncast | exit |
| Lock Screen | none |
| Suspend | sleep |
| Log Out | logout, sign out |
| Reboot | restart |
| Power Off | shutdown |

## The comparator keeps eleven of TinyCast's rules

TinyCast orders two shown entries by the first of 14 rules that separates them. lyncast keeps 11. Aliases (rules 1 and 6) come after the MVP, and lyncast has no boosted command (rule 2). TinyCast's numbers stay, so this list and TinyCast's doc read together.

3. The title or alternate title equals the query, and L is more than 3.
4. A past search term equals the query.
5. The subtitle equals the query, at any length.
7. A past search term starts with the query.
8. The query starts with a past search term of 3 or more characters and runs at most 3 characters past it.
9. The best score over the title, alternate title, and subtitle. Keywords never count.
10. Frecency.
11. The title's own score.
12. The title or alternate title starts with the query.
13. Kind priority. Apps come before commands.
14. The name, compared numerically.

Rules 3, 4, and 5 have sub-orders. Two entries that both meet rule 3 go by search-term strength (an exact term, then a term that starts with the query, then a term the query runs past), then frecency. Both meeting rule 4 go by frecency. Both meeting rule 5 go by frecency, then the title's own score. The tiebreak settles the rest and is frecency, then kind priority, then the name.

`rank/key.rs` encodes this as one value per shown entry, `RankKey { standing, tiebreak }`, with a derived `Ord`. `Standing` is an enum whose variants are rules 3, 4, and 5, then one variant for rules 7 to 12, and each variant holds only its own sub-order. The first field that differs between two keys is the first rule that separates the entries.

The name order is computed once, when the index is built. It compares the lowercased fold of the titles, with runs of ASCII digits compared by value, so `App 9` comes before `App 10`. Then it compares the raw titles, then the keys, so no two entries tie. It does no locale collation, so `Ångström` sorts as `angstrom`. The index stores its entries in this order, and rule 14 compares two positions.

Each block below is tagged with the rule it pins. Its entries are built so that the order would flip without that rule, or without the part of it that the prose above the block names.

```rank rule=3
app firefox.desktop | Name=Firefox
app firefox-developer-edition.desktop | Name=Firefox Developer Edition
visit app:firefox-developer-edition.desktop 99 firefox
now 100
query firefox
results => Firefox, Firefox Developer Edition
```

The title is trimmed before the comparison, because GLib keeps a value's trailing whitespace. `Name=Firefox\s` ends in a space and still equals `firefox`. The row's title keeps the space.

```rank rule=3
app firefox.desktop | Name=Firefox\s
app firefox-developer-edition.desktop | Name=Firefox Developer Edition
visit app:firefox-developer-edition.desktop 99 firefox
now 100
query firefox
results => Firefox , Firefox Developer Edition
```

Rule 3 waits until the query is past 3 characters, so at `zed` the exact past term wins.

```rank rule=3
app zed.desktop | Name=Zed
app zed-settings-sync.desktop | Name=Zed Settings Sync
visit app:zed-settings-sync.desktop 99 zed
now 100
query zed
results => Zed Settings Sync, Zed
```

At 4 characters rule 3 applies, so the exact title Code comes before Code Tools' exact past term.

```rank rule=3
app code.desktop | Name=Code
app code-tools.desktop | Name=Code Tools
visit app:code-tools.desktop 99 code
now 100
query code
results => Code, Code Tools
```

L counts no separators, so `a b c` has L 3. The exact title A B C waits, and Abc Tool's exact past term wins.

```rank rule=3
app abc.desktop | Name=A B C
app abc-tool.desktop | Name=Abc Tool
visit app:abc-tool.desktop 99 a b c
now 100
query a b c
results => Abc Tool, A B C
```

An alternate title meets rule 3 too. Konsole's unlocalized `Name` is Terminal, so Konsole comes before Terminal Tools' exact past term.

```rank rule=3
session LANG=de_DE.UTF-8
app konsole.desktop | Name=Terminal | Name[de]=Konsole
app termtools.desktop | Name=Terminal Tools
visit app:termtools.desktop 99 terminal
now 100
query terminal
results => Konsole, Terminal Tools
```

When both entries meet rule 3, search-term strength decides before frecency. Terminal's term starts with the query, and the query only runs past Konsole's, although Konsole was opened twice.

```rank rule=3
session LANG=de_DE.UTF-8
app terminal.desktop | Name=Terminal
app konsole.desktop | Name=Terminal | Name[de]=Konsole
visit app:konsole.desktop 98 termi
visit app:konsole.desktop 98 termi
visit app:terminal.desktop 99 terminal emulator
now 100
query terminal
results => Terminal, Konsole
```

An exact term is stronger than one that starts with the query. Both entries were opened once, and the name would put Konsole first.

```rank rule=3
session LANG=de_DE.UTF-8
app terminal.desktop | Name=Terminal
app konsole.desktop | Name=Terminal | Name[de]=Konsole
visit app:terminal.desktop 99 terminal
visit app:konsole.desktop 99 terminal emulator
now 100
query terminal
results => Terminal, Konsole
```

A term the query runs past is still stronger than no term. Terminal was opened twice with no term, so frecency alone would put it first.

```rank rule=3
session LANG=de_DE.UTF-8
app terminal.desktop | Name=Terminal
app konsole.desktop | Name=Terminal | Name[de]=Konsole
visit app:terminal.desktop 97
visit app:terminal.desktop 97
visit app:konsole.desktop 99 termi
now 100
query terminal
results => Konsole, Terminal
```

Both entries meet rule 7, and only Files meets rule 4. Firefox was opened later and twice, so frecency would put it first. A visit's query is quoted as a query line may be, so `"fi"` is the term `fi`.

```rank rule=4
app files.desktop | Name=Files
app firefox.desktop | Name=Firefox
visit app:files.desktop 90 "fi"
visit app:firefox.desktop 99 fire
visit app:firefox.desktop 99 fire
now 100
query fi
results => Files, Firefox, Open Config
```

Both entries meet rule 4, so frecency decides.

```rank rule=4
app firefox.desktop | Name=Firefox
app files.desktop | Name=Files
visit app:firefox.desktop 99 fi
visit app:files.desktop 97 fi
now 100
query fi
results => Firefox, Files, Open Config
```

An exact past term comes before an exact subtitle.

```rank rule=4
app webby.desktop | Name=Webby
app firefox.desktop | Name=Firefox | GenericName=Web
visit app:webby.desktop 99 web
now 100
query web
results => Webby, Firefox
```

```rank rule=5
app firefox.desktop | Name=Firefox | GenericName=Web Browser
app web-browser-tools.desktop | Name=Web Browser Tools
visit app:web-browser-tools.desktop 99 web browser tools
now 100
query web browser
results => Firefox, Web Browser Tools
```

Both entries meet rule 5. With no visits, the title's own score decides. Xfiles's title aligns with `files`, and Nautilus's does not.

```rank rule=5
app nautilus.desktop | Name=Nautilus | GenericName=Files
app xfiles.desktop | Name=Xfiles | GenericName=Files
now 100
query files
results => Xfiles, Nautilus
```

Frecency comes before the title's score.

```rank rule=5
app nautilus.desktop | Name=Nautilus | GenericName=Files
app xfiles.desktop | Name=Xfiles | GenericName=Files
visit app:nautilus.desktop 99
now 100
query files
results => Nautilus, Xfiles
```

The subtitle is trimmed too, so `GenericName=VLC\s` equals `vlc`. VLC's title equals `vlc` as well, but L is 3, so rule 3 does not apply. Without rule 5, both best fits would be exact, and frecency would put VLC first. Without the trim, Media Player's subtitle would score 8, and VLC's exact title would win rule 9. No command aligns with `vlc`.

```rank rule=5
app media-player.desktop | Name=Media Player | GenericName=VLC\s
app vlc.desktop | Name=VLC
visit app:vlc.desktop 99
now 100
query vlc
results => Media Player, VLC
```

A past term that starts with the query beats a higher score, 4 against Firefox's 6. Open Config shows because `fi` runs inside `Config`.

```rank rule=7
app firefox.desktop | Name=Firefox
app profile-manager.desktop | Name=Profile Manager
visit app:profile-manager.desktop 99 fire
now 100
query fi
results => Profile Manager, Firefox, Open Config
```

A past term still steers 10 days and 17 days after the visit.

```rank rule=7
app firefox.desktop | Name=Firefox
app profile-manager.desktop | Name=Profile Manager
visit app:profile-manager.desktop 90 fire
now 100
query fi
results => Profile Manager, Firefox, Open Config
```

```rank rule=7
app firefox.desktop | Name=Firefox
app profile-manager.desktop | Name=Profile Manager
visit app:profile-manager.desktop 83 fire
now 100
query fi
results => Profile Manager, Firefox, Open Config
```

It stops steering 18 days after the visit, although the score is still above 1. Rule 9 decides, and frecency puts Profile Manager above Open Config.

```rank rule=7
app firefox.desktop | Name=Firefox
app profile-manager.desktop | Name=Profile Manager
visit app:profile-manager.desktop 82 fire
now 100
query fi
results => Firefox, Profile Manager, Open Config
```

A term that starts with the query comes before one that the query runs past. Without rules 7 and 8, the name would put Firefox first.

```rank rule=7
app firewall.desktop | Name=Firewall
app firefox.desktop | Name=Firefox
visit app:firewall.desktop 99 firewall
visit app:firefox.desktop 99 fir
now 100
query fire
results => Firewall, Firefox
```

```rank rule=8
app firefox.desktop | Name=Firefox
app firefox-developer-edition.desktop | Name=Firefox Developer Edition
visit app:firefox-developer-edition.desktop 98
visit app:firefox-developer-edition.desktop 98
visit app:firefox.desktop 99 fir
now 100
query firef
results => Firefox, Firefox Developer Edition
```

`firefo` runs 3 past the term `fir`, so rule 8 holds. `firefox` runs 4 past, so it does not, and the name decides.

```rank rule=8
app firefox-esr.desktop | Name=Firefox ESR
app firefox-nightly.desktop | Name=Firefox Nightly
visit app:firefox-esr.desktop 99
visit app:firefox-nightly.desktop 99 fir
now 100
query firefo
results => Firefox Nightly, Firefox ESR
```

```rank rule=8
app firefox-esr.desktop | Name=Firefox ESR
app firefox-nightly.desktop | Name=Firefox Nightly
visit app:firefox-esr.desktop 99
visit app:firefox-nightly.desktop 99 fir
now 100
query firefox
results => Firefox ESR, Firefox Nightly
```

A term needs 3 characters to count for rule 8. Profile Reader's `fi` has 2, so rule 9 decides.

```rank rule=8
app firefox.desktop | Name=Firefox
app profile-reader.desktop | Name=Profile Reader
visit app:firefox.desktop 99
visit app:profile-reader.desktop 99 fi
now 100
query fire
results => Firefox, Profile Reader
```

Rule 8 comes before rule 9. `firef` runs 2 past Xfirefox's term `fir`. Firefox's title scores 12 against Xfirefox's 10, and Firefox was opened twice.

```rank rule=8
app firefox.desktop | Name=Firefox
app xfirefox.desktop | Name=Xfirefox
visit app:firefox.desktop 98
visit app:firefox.desktop 98
visit app:xfirefox.desktop 99 fir
now 100
query firef
results => Xfirefox, Firefox
```

Arc's keyword `browser` scores 10 against Web Browser Tool's 9, but a keyword never ranks. Arc shows and comes last.

```rank rule=9
app arc.desktop | Name=Arc | Keywords=browser;
app web-browser-tool.desktop | Name=Web Browser Tool
now 100
query brow
results => Web Browser Tool, Arc
```

Rule 11 gives this order too, so this block pins the points rather than rule 9 alone. `Code` is a word start and scores 9, and `code` inside `Barcode` scores 8.

```rank rule=9
app visual-studio-code.desktop | Name=Visual Studio Code
app barcode-scanner.desktop | Name=Barcode Scanner
now 100
query code
results => Visual Studio Code, Barcode Scanner
```

A subtitle counts for rule 9. Konsole's scores 10 and beats Xterm's title, 8, although rule 11 would put Xterm first.

```rank rule=9
app konsole.desktop | Name=Konsole | GenericName=Terminal
app xterm.desktop | Name=Xterm
now 100
query term
results => Konsole, Xterm
```

So does an alternate title.

```rank rule=9
session LANG=de_DE.UTF-8
app konsole.desktop | Name=Terminal | Name[de]=Konsole
app xterm.desktop | Name=Xterm
now 100
query term
results => Konsole, Xterm
```

Both entries score 10, Konsole through its subtitle. Frecency puts Konsole first, although Terminal's title would win rules 11 and 12.

```rank rule=10
app konsole.desktop | Name=Konsole | GenericName=Terminal
app terminal.desktop | Name=Terminal
visit app:konsole.desktop 99
now 100
query term
results => Konsole, Terminal
```

Frecency also comes before Xterm's title score, 8.

```rank rule=10
app konsole.desktop | Name=Konsole | GenericName=Terminal
app xterm.desktop | Name=Xterm | GenericName=Terminal
visit app:konsole.desktop 99
now 100
query term
results => Konsole, Xterm
```

Both subtitles score 10. Xterm's own title scores 8, and Konsole's does not align.

```rank rule=11
app konsole.desktop | Name=Konsole | GenericName=Terminal
app xterm.desktop | Name=Xterm | GenericName=Terminal
now 100
query term
results => Xterm, Konsole
```

Rule 11 reads only the title. Zubehör scores 6 through its alternate title, App Store, and its own title does not align. So AirPort Utility comes first, although only Zubehör starts with `ap`.

```rank rule=11
session LANG=de_DE.UTF-8
app zubehoer.desktop | Name=App Store | Name[de]=Zubehör
app airport-utility.desktop | Name=AirPort Utility
now 100
query ap
results => AirPort Utility, Zubehör, Reload Apps
```

App Store and AirPort Utility both score 6, AirPort Utility by skipping to `Port`. Only App Store starts with `ap`.

```rank rule=12
app app-store.desktop | Name=App Store
app airport-utility.desktop | Name=AirPort Utility
now 100
query ap
results => App Store, AirPort Utility, Reload Apps
```

An alternate title that starts with the query counts too.

```rank rule=12
session LANG=de_DE.UTF-8
app app-store.desktop | Name=App Store | Name[de]=AirPort Utility
app airport-tool.desktop | Name=AirPort Tool
now 100
query ap
results => AirPort Utility, AirPort Tool, Reload Apps
```

```rank rule=13
app logseq.desktop | Name=Logseq
now 100
query log
results => Logseq, Log Out
```

```rank rule=14
app firefox-10.desktop | Name=Firefox 10
app firefox-9.desktop | Name=Firefox 9
now 100
query fire
results => Firefox 9, Firefox 10
```

A command shows through its keyword, as an app does, so `shutdown` finds Power Off.

```rank keywords
now 100
query shutdown
results => Power Off
```

An app with a subtitle has two more keywords, its title and subtitle joined both ways, as TinyCast's keywords do. `Firefox Web Browser` scores 22 on `firefox web`, but a keyword never ranks, so Firefox Wallet Bridge comes first on its title's 21.

```rank keywords
app firefox.desktop | Name=Firefox | GenericName=Web Browser
app firefox-wallet-bridge.desktop | Name=Firefox Wallet Bridge
now 100
query firefox web
results => Firefox Wallet Bridge, Firefox
```

The other join is `Web Browser Firefox`.

```rank keywords
app firefox.desktop | Name=Firefox | GenericName=Web Browser
now 100
query web browser firefox
results => Firefox
```

The query loses its separators, so `fire-fox` finds Firefox.

```rank separators
app firefox.desktop | Name=Firefox
now 100
query fire-fox
results => Firefox
```

So does a tab or a newline inside the query.

```rank separators
app firefox.desktop | Name=Firefox
now 100
query fire\u{9}fox
results => Firefox
```

```rank separators
app firefox.desktop | Name=Firefox
now 100
query fire\u{A}fox
results => Firefox
```

A query of only separators is a typed query that matches nothing.

```rank separators
app firefox.desktop | Name=Firefox
now 100
query -
results => none
```

## Frecency halves every 10 days

lyncast keeps TinyCast's learned ranking. Each visited entry has a frecency anchor, the time it was last opened, and its last 3 distinct search terms:

```
score(t) = max(1, e^(λ·(anchor − t)))       λ = ln 2 per 10 days
a visit:  anchor = t + ln(score(t) + 100) / λ, so the score rises by 100
```

The anchor is stored instead of the score, because a later anchor is always a higher score. Ranking compares anchors and never computes the exponential. An anchor at or before now scores 1, the same as an entry never opened. Anchors are whole milliseconds since the Unix epoch, so step 5 can store them exactly.

Search terms are stored folded, lowercased, and trimmed. An empty query records no term. Terms steer rules 3, 4, 7, and 8 only while the score is above 1 and the entry was opened in the last 17 days.

The clock is always an argument. `lyncast-core` never reads it, and the binary passes the system time. Nothing persists in step 4, because nothing records a visit before `activate` exists. Step 5's daemon owns one `Visits` and writes it to `$XDG_DATA_HOME/lyncast/ranking.json` ([decision log](../decisions.md#app-index-and-ranking)).

`visits` lists the times of visits with an empty query, as offsets from a fixed clock, and `at` is when the score is read. One visit puts the anchor ln(101)/λ ahead, about 66.6 days.

```frecency
visits 0 | at 0 => 101.000
visits 0 | at 10d => 50.500
visits 0 | at 66d => 1.041
visits 0 | at 67d => 1.000
visits 0, 0 | at 0 => 201.000
visits 0, 10d | at 20d => 75.250
```

The queries are recorded on one entry in order, and the line ends with its terms, most recent first.

```terms
"Fire" "fire" "FIREFOX" "ｆｉｒｅ" "" "web" "mail" => mail | web | fire
"Fire" "web" "fire" => fire | web
```

A repeated term moves to the front and keeps one place, so the second line holds two terms. `Fire` and `fire` fold to one term.

## The empty query shows Suggestions, then Commands

Suggestions holds at most 5 rows. It takes the apps and commands whose frecency is above 1, by the tiebreak. While it has fewer than 5, it adds Clipboard History, then apps in name order, and skips any it already holds. On a fresh install it shows Clipboard History and the first 4 apps.

Commands lists every built-in command that Suggestions does not hold, in name order. No key appears twice in the list (user decision, 2026-10-15).

```rank empty-list
app alpha.desktop | Name=Alpha
app beta.desktop | Name=Beta
app delta.desktop | Name=Delta
app epsilon.desktop | Name=Epsilon
app gamma.desktop | Name=Gamma
now 100
query ""
suggestions => Clipboard History, Alpha, Beta, Delta, Epsilon
commands => Lock Screen, Log Out, Open Config, Power Off, Quit lyncast, Reboot, Reload Apps, Suspend
```

Gamma was opened twice, so it comes first. Beta and Reboot have equal anchors, and the app comes first. Delta's only visit was 99 days ago, so its score is back to 1.

```rank empty-list
app alpha.desktop | Name=Alpha
app beta.desktop | Name=Beta
app delta.desktop | Name=Delta
app gamma.desktop | Name=Gamma
visit app:gamma.desktop 99
visit app:gamma.desktop 99
visit app:beta.desktop 99
visit cmd:reboot 99 reb
visit app:delta.desktop 1
now 100
query ""
suggestions => Gamma, Beta, Reboot, Clipboard History, Alpha
commands => Lock Screen, Log Out, Open Config, Power Off, Quit lyncast, Reload Apps, Suspend
```

Frecency comes before kind priority. Reboot was opened after Beta, so the command comes first.

```rank empty-list
app alpha.desktop | Name=Alpha
app beta.desktop | Name=Beta
visit app:beta.desktop 90
visit cmd:reboot 99
now 100
query ""
suggestions => Reboot, Beta, Clipboard History, Alpha
commands => Lock Screen, Log Out, Open Config, Power Off, Quit lyncast, Reload Apps, Suspend
```

Zeal and Reboot have equal anchors. The app comes first, although the name would put Reboot first.

```rank empty-list
app zeal.desktop | Name=Zeal
visit app:zeal.desktop 99
visit cmd:reboot 99
now 100
query ""
suggestions => Zeal, Reboot, Clipboard History
commands => Lock Screen, Log Out, Open Config, Power Off, Quit lyncast, Reload Apps, Suspend
```

Whitespace is the empty query.

```rank whitespace
app firefox.desktop | Name=Firefox
now 100
query "   "
suggestions => Clipboard History, Firefox
commands => Lock Screen, Log Out, Open Config, Power Off, Quit lyncast, Reboot, Reload Apps, Suspend
```

The empty query is decided after the fold, so text that folds to nothing is empty too, such as a lone soft hyphen.

```rank whitespace
app firefox.desktop | Name=Firefox
now 100
query \u{AD}
suggestions => Clipboard History, Firefox
commands => Lock Screen, Log Out, Open Config, Power Off, Quit lyncast, Reboot, Reload Apps, Suspend
```

## The scan reads desktop entries the way the spec and GLib do

lyncast parses desktop entries itself. freedesktop-desktop-entry 0.8.3 rejected the whole file for six inputs: the spec-legal `Keywords=a\;b;`, a UTF-8 BOM, an indented comment, a localized key before a base key that ends its group when another group follows, a localized key without its base key, and `Comment=C:\Users`. GLib reads every one but the BOM. The crate's Exec parser split `sh -c "echo a b"` on every space (measured, [decision log](../decisions.md#stack)).

The binary looks in `$XDG_DATA_HOME/applications` (default `~/.local/share/applications`), then in each `$XDG_DATA_DIRS` entry's `applications` (default `/usr/local/share:/usr/share`). It ignores a relative path, as the XDG Base Directory spec asks, and a path that is not UTF-8, because an ID and `%k` are text. An ignored `XDG_DATA_HOME` falls back to `~/.local/share`, and an ignored `XDG_DATA_DIRS` entry drops only that entry. GLib 2.82.2 reads both kinds of path, a relative one from its working directory (measured on the dev box, 2026-10-16). It drops trailing slashes, as GLib's `g_build_filename` does, so a `%k` path holds no `//`. It walks each directory recursively in name order and follows symlinked directories. It does not follow a link back to a directory it is already inside, so a cycle ends. A directory it reached through a link is still walked under its own path, so its files show under both IDs, as GLib 2.82.2 lists them. GLib has no cycle check. It follows `loop -> .` until the kernel's limit of 40 links and lists each file again under a longer ID (measured on the dev box, 2026-10-16).

A desktop-file ID is the path under `applications/` with `/` replaced by `-`. It keeps `.desktop`, so `mozilla/firefox.desktop` is `mozilla-firefox.desktop` and its key is `app:mozilla-firefox.desktop`.

The first file for an ID decides for that ID, as the spec says. The name decides before the file is read, so a later directory's file with the same ID is never read, even when the first file cannot be read, does not parse, or hides its entry. This is how `Hidden=true` in `~/.local/share/applications` deletes a system entry. GLib 2.82.2 masks the same way (measured on the dev box, 2026-10-16). Each of these in the user's directory hid the system entry with the same ID: a file with a line that is not `key=value`, one with no `[Desktop Entry]` group, one with `Hidden=true`, one whose Exec program is missing, an empty file, an unreadable file, a broken symlink, a symlink to `/dev/zero`, and a directory with a `.desktop` name. The binary reads a file only when it is a regular file after symlinks and holds at most 1 MiB, so a link to a device cannot make the scan read without bound, and a FIFO cannot block it. GLib opens a file before it checks that it is regular. So it skips a device such as `/dev/zero`, but it blocks on a FIFO until a writer opens it. GLib also reads a longer file, so a desktop file over 1 MiB shows in GLib and only takes its ID in lyncast (measured, 2026-10-16). A byte that is not UTF-8 reads as U+FFFD, so a bad byte in one translation does not hide the user's entry. GLib loads such a file too and fails only the key that holds the byte.

`lyncast_core::desktop::parse` turns one file's text into a verdict. It checks the first six of these in this order, the binary checks the last two, and the first that applies hides the entry:

1. `Hidden=true`. A file that holds only this line does not parse, and it still hides its ID.
2. `Type` is missing or is not `Application`.
3. `NoDisplay=true`.
4. `OnlyShowIn` and `NotShowIn`, against `XDG_CURRENT_DESKTOP` split on `:`, compared case-sensitively. This follows GLib. The parser walks the current desktops in order. A desktop in `OnlyShowIn` shows the entry, and one in `NotShowIn` hides it. If none is in either list, the entry shows only when it has no `OnlyShowIn`.
5. `Name` is missing.
6. `Exec` is missing, or does not tokenize. lyncast does not support D-Bus activation.
7. `TryExec` is not found.
8. The Exec program, the first argument of the tokenized `Exec`, is not found.

A program is found when an absolute path is a file this user may execute, or a bare name is one in the entry's `Path` directory or in a `PATH` directory. The binary looks in `Path` first, and only when it is absolute. It reads `PATH` as bytes, so an entry that is not UTF-8 hides nothing. An unset `PATH` searches `/bin:/usr/bin:.`. A relative `PATH` entry, such as `.` or the one empty entry of an empty `PATH`, names a directory under `Path`, or under the working directory when the desktop entry has no `Path`. An empty `Path` counts as none. GLib 2.82.2's `g_desktop_app_info_new_from_filename` searches the same directories (measured on the dev box, 2026-10-16). The binary asks `access(X_OK)`, as GLib does, so a file whose execute bit is only its owner's hides the entry from everyone else. GLib 2.82.2 hides an entry whose Exec program is missing too, whether the program is a bare name or an absolute path, and also when `TryExec` is found. It looks up only the first argument, so `Exec=env FOO=1 ghost` shows (measured on the dev box, 2026-10-16). A directory is never a program, so a `TryExec` or an Exec program that names one hides the entry. GLib 2.82.2 hid `TryExec=/usr` and `Exec=/usr %u` too (measured on the dev box, 2026-10-16). These are the known differences from GLib, measured on GLib 2.82.2 on the dev box the same day:

- A relative path with a `/` is never found, as a `TryExec` or as the Exec program. GLib looks it up from the `Path` key's directory, or else from its own working directory, and then under each `PATH` directory, so `Exec=./app` and `Exec=x/app` show when `$PATH/app` and `$PATH/x/app` exist. With the CLI tree's files, `TryExec=bin/firefox` and `Exec=bin/firefox %u` showed in GLib, whose working directory held `bin/firefox`, and they hide in lyncast.
- lyncast looks up the first argument after the field codes expand, and GLib before, so `Exec=%U app` shows in lyncast and not in GLib.
- GLib lists an entry with no `Name` as `Unnamed`, and an entry with no `Exec`, where items 5 and 6 hide them.

`Terminal=true` apps show like any other (user decision, 2026-10-15). The entry records the flag, and step 8 decides which terminal runs them.

The binary reads a list of locale names. A `LANGUAGE` that is not empty gives the list, split on `:`, even when `LC_ALL` or `LANG` is `C`. Otherwise the list is the first non-empty value of `LC_ALL`, `LC_MESSAGES`, and `LANG`, as POSIX orders them. That locale is not added after a `LANGUAGE` list. GLib 2.82.2's `g_get_language_names` does the same. `LANGUAGE=de:fr` with `LANG=en_US.UTF-8` gave `de fr C`, `LANGUAGE=de` gave `de C` beside `LC_ALL=C`, `LC_ALL=fr_FR.UTF-8`, `LC_MESSAGES=fr_FR.UTF-8`, or `LANG=C`, and an empty `LANGUAGE` counted as unset (measured on the dev box, 2026-10-16).

For each name in turn, the parser drops the encoding from `lang_COUNTRY.ENCODING@MODIFIER` and tries `Key[lang_COUNTRY@MODIFIER]`, `Key[lang_COUNTRY]`, `Key[lang@MODIFIER]`, and `Key[lang]`, as the spec orders them. A candidate an earlier name gave is not tried again, and `Key` comes last. `C` and `POSIX` give no candidates. An empty localized value falls through to the next candidate. These are the known differences from GLib, measured on GLib 2.82.2 on the dev box (2026-10-16):

- GLib tries `lang@MODIFIER` before `lang_COUNTRY`, so `sr_RS.UTF-8@latin` reads `Name[sr@latin]` before `Name[sr_RS]`.
- GLib expands an alias such as `german` from `locale.alias`, and lyncast reads such a name as the language `german`.
- GLib tries `Key[C]` last in every locale and `Key[POSIX]` under `POSIX`, so `Name[C]=CName` names the app `CName` in GLib and lyncast shows `Name`.
- GLib tries the names with their encoding, so under `LANG=de_DE.UTF-8` it reads `Name[de_DE.UTF-8]` and `Name[de.UTF-8]`, which lyncast never tries, as the spec drops the encoding.

The parser reads what GLib 2.82 reads, checked against `g_key_file_load_from_data` on the dev box. It ignores leading whitespace before `#` and on key lines, accepts a localized key before or without its base key, merges a repeated group into the first, keeps the last value of a repeated key, and rejects the whole file for a line that is not a comment, a group, or `key=value`. String values decode `\s`, `\n`, `\t`, `\r`, and `\\`, and lists also decode `\;`. Where it reads a file differently, it is more lenient than GLib, so that an app still shows. These are the known differences, measured on GLib 2.82.2 on the dev box (2026-10-16). The lists of names and escapes in them are not exhaustive:

- It skips a leading BOM, where GLib fails the whole file.
- An unknown escape, such as `\U`, keeps its backslash, and so does a lone backslash at the end of a value, where GLib fails that key. `Name=Trail Name\` shows as `Trail Name\` in lyncast and as `Unnamed` in GLib.
- An empty localized value falls through to the next candidate, where GLib returns the empty string.
- A localized list that holds `\;` reads as written, where `g_key_file_get_locale_string_list` fails that key and falls back to the unlocalized list.
- Key and group names that GLib rejects read as written, where GLib fails the whole file. For example, the keys `Name[de DE]`, `X-Foo]`, `Na[me`, `Name[de!]`, `Name[de/x]`, and `Name[d[e]`, and the groups `[]`, `[A]B]`, `[A[B]`, and a group name that holds a tab or U+0001. GLib reads `X Foo=x` as a key, as lyncast does.
- An `Encoding` key in `[Desktop Entry]` reads as any other key, where GLib fails the whole file for every value but `UTF-8`, which it compares ignoring case. `Encoding=Legacy-Mixed` and `Encoding=ISO-8859-1` each make `g_desktop_app_info_new_from_filename` return NULL, and the app shows in lyncast. GLib accepts `Encoding=utf-8` and an `Encoding` in a later group.
- The binary reads a byte that is not UTF-8 as U+FFFD, where GLib fails the key that holds it.
- A NUL in a value is kept, with the text after it, where GLib ends the value at the NUL. A `Name` of `Nul`, a NUL, and `Tail` shows as `Nul` in GLib and keeps all eight characters in lyncast. `a_nul_in_a_value_is_kept` in `tests/desktop.rs` pins this, because a `parsed` line cannot hold a NUL.
- It drops a `\r` that ends the last line, where GLib keeps it in the value. `Exec=app` and `\r` at the end of the file shows in lyncast and hides in GLib, which looks for the program `app\r`.

In `TryExec`, `OnlyShowIn`, and `NotShowIn`, some of these readings make lyncast stricter than GLib. GLib fails a key that holds an unknown escape, a lone backslash at its end, or a byte that is not UTF-8, and then ignores that key, and it ends a value at a NUL. lyncast reads the value as written, so such a key can hide an app that GLib shows. `TryExec=a\pp` with `app` on `PATH`, `OnlyShowIn=GNOME;\q;`, a `NotShowIn` that holds `niri;` and a byte that is not UTF-8, `TryExec=sh` followed by a NUL and `x` with `/bin` on `PATH`, and `OnlyShowIn=niri` followed by a NUL and `x;` each show in GLib on niri and hide in lyncast (measured on the dev box, 2026-10-16).

Desktop Actions come in `Actions=` order. Each needs a `[Desktop Action <id>]` group with a `Name` and an `Exec` that tokenizes. Otherwise that action is skipped and the app still shows. `%c`, `%i`, and `%k` in an action expand to the app's values.

An `entry` block holds a file's text, and the `parsed` block after it holds the verdict. The attributes give the locale and `XDG_CURRENT_DESKTOP`. The file is parsed as `/usr/share/applications/test.desktop`. The `profile` action has no group, and `broken` has an Exec that does not tokenize, so both are skipped.

```entry locale=de_DE.UTF-8 desktop=niri
[Desktop Entry]
Type=Application
Name=Firefox
GenericName=Web Browser
GenericName[de]=Webbrowser
Keywords=Internet;WWW;
Exec=firefox %u
TryExec=firefox
Icon=firefox
Actions=new-window;new-private-window;profile;broken;

[Desktop Action new-window]
Name=New Window
Exec=firefox --new-window %u

[Desktop Action new-private-window]
Name=New Private Window
Name[de]=Neues privates Fenster
Exec=firefox --private-window %u

[Desktop Action broken]
Name=Broken
Exec=firefox "--unterminated
```

```parsed
name: Firefox
unlocalized name: Firefox
generic name: Webbrowser
keywords: Internet | WWW
icon: firefox
argv: ["firefox"]
terminal: false
try-exec: firefox
action new-window: New Window ["firefox", "--new-window"]
action new-private-window: Neues privates Fenster ["firefox", "--private-window"]
```

Every input that freedesktop-desktop-entry rejected parses here. `GenericName[de]` has no base key, and `Name[de]` comes before a `Name` that ends its group, with another group after it. The file's `\\"` reaches the tokenizer as `\"`.

```entry locale=de_DE.UTF-8 desktop=niri
\u{FEFF}[Desktop Entry]
  # an indented comment
Type=Application
GenericName[de]=Taschenrechner
Keywords[de]=Mathe\;Zahlen;Rechnen;
Comment[de]=Rechnet
Comment=C:\Users
Exec=sh -c "echo \\"a b\\""
Keywords=math;
Name[de]=Rechner
Name=Calculator

[X-Calculator]
Mode=basic
```

```parsed
name: Rechner
unlocalized name: Calculator
generic name: Taschenrechner
keywords: Mathe;Zahlen | Rechnen | math
icon: none
argv: ["sh", "-c", "echo \"a b\""]
terminal: false
try-exec: none
```

An empty localized value falls through, here to the base `GenericName`. An unknown escape keeps its backslash, and so does a lone backslash at the end.

```entry locale=de_DE.UTF-8 desktop=niri
[Desktop Entry]
Type=Application
Name=Files
GenericName[de]=
GenericName=C:\Users\
Exec=nautilus
```

```parsed
name: Files
unlocalized name: Files
generic name: C:\Users\
keywords: none
icon: none
argv: ["nautilus"]
terminal: false
try-exec: none
```

A file with only a localized `Name` uses it for the alternate title too. `de_DE.UTF-8` drops its encoding, so `Name[de_DE]` comes before `Name[de]`.

```entry locale=de_DE.UTF-8 desktop=niri
[Desktop Entry]
Type=Application
Name[de]=Rechner
Name[de_DE]=Taschenrechner
Exec=calc
```

```parsed
name: Taschenrechner
unlocalized name: Taschenrechner
generic name: none
keywords: none
icon: none
argv: ["calc"]
terminal: false
try-exec: none
```

`sr_RS.UTF-8@latin` tries `sr_RS@latin`, then `sr_RS`, then `sr@latin`, then `sr`. Each localized key below has two neighbouring candidates, and the earlier one in this order wins. A terminal app shows.

```entry locale=sr_RS.UTF-8@latin desktop=niri
[Desktop Entry]
Type=Application
Name=System Monitor
Name[sr_RS]=Системски монитор
Name[sr_RS@latin]=Sistemski monitor
GenericName[sr@latin]=Nadzor sistema
GenericName[sr_RS]=Надзор система
Keywords[sr]=процес;
Keywords[sr@latin]=proces;
Exec=htop
Terminal=true
```

```parsed
name: Sistemski monitor
unlocalized name: System Monitor
generic name: Надзор система
keywords: proces
icon: none
argv: ["htop"]
terminal: true
try-exec: none
```

A repeated key keeps its last value, as GLib does.

```entry locale=C desktop=niri
[Desktop Entry]
Type=Application
Name=Old Name
Exec=old
Name=New Name
Exec=new
```

```parsed
name: New Name
unlocalized name: New Name
generic name: none
keywords: none
icon: none
argv: ["new"]
terminal: false
try-exec: none
```

GLib 2.82.2 reads each line below the same way. It ignores whitespace after a group header, before a key, and around `=`. A repeated localized key keeps its last value, `\t` and `\r` decode, and a boolean drops its trailing ASCII whitespace. An empty `TryExec` is ignored, and a desktop in both `OnlyShowIn` and `NotShowIn` shows the entry.

```entry locale=de_DE.UTF-8 desktop=niri
[Desktop Entry]\u{20}\u{9}
  Type=Application
Name = Spaced
Name[de]=Erste
Name[de]=Zweite
Exec = app "a\tb\rc"
TryExec=
Terminal=true\u{20}
OnlyShowIn=niri;
NotShowIn=niri;
```

```parsed
name: Zweite
unlocalized name: Spaced
generic name: none
keywords: none
icon: none
argv: ["app", "a\tb\rc"]
terminal: true
try-exec: none
```

A repeated group continues the first group of that name, as GLib merges them.

```entry locale=C desktop=niri
[Desktop Entry]
Type=Application
Name=Files

[Desktop Action new-window]
Name=New Window

[Desktop Entry]
Exec=nautilus
```

```parsed
name: Files
unlocalized name: Files
generic name: none
keywords: none
icon: none
argv: ["nautilus"]
terminal: false
try-exec: none
```

A line that is not a comment, a group, or `key=value` makes the whole file unparseable.

```entry locale=C desktop=niri
[Desktop Entry]
Type=Application
Name=Files
Exec=nautilus
garbage
```

```parsed
unparseable: BadLine(5)
```

So does a key before the first group, as in GLib.

```entry locale=C desktop=niri
X-Before=1
[Desktop Entry]
Type=Application
Name=Files
Exec=nautilus
```

```parsed
unparseable: NoDesktopEntryGroup
```

The first group must be `[Desktop Entry]`, as GLib's `g_desktop_app_info_new_from_filename` requires.

```entry locale=C desktop=niri
[Other]
A=b

[Desktop Entry]
Type=Application
Name=Files
Exec=nautilus
```

```parsed
unparseable: NoDesktopEntryGroup
```

A file that holds only `Hidden=true` has no group either. The binary still lets such a file decide its ID, so it hides the system entry too.

```entry locale=C desktop=niri
Hidden=true
```

```parsed
unparseable: NoDesktopEntryGroup
```

A line may end in `\r`, which the parser drops, and `\s` in a value is a space. GLib 2.82.2 reads both the same way, but it drops `\r` only before `\n`.

```entry locale=C desktop=niri
[Desktop Entry]\u{D}
Type=Application\u{D}
Name=Disk\sUsage\u{D}
Exec=baobab\u{D}
```

```parsed
name: Disk Usage
unlocalized name: Disk Usage
generic name: none
keywords: none
icon: none
argv: ["baobab"]
terminal: false
try-exec: none
```

In an action, `%i`, `%c`, and `%k` expand to the app's Icon, Name, and path.

```entry locale=C desktop=niri
[Desktop Entry]
Type=Application
Name=Editor
Icon=editor
Exec=editor
Actions=new-window;

[Desktop Action new-window]
Name=New Window
Exec=editor --new-window %i --class=%c %k
```

```parsed
name: Editor
unlocalized name: Editor
generic name: none
keywords: none
icon: editor
argv: ["editor"]
terminal: false
try-exec: none
action new-window: New Window ["editor", "--new-window", "--icon", "editor", "--class=Editor", "/usr/share/applications/test.desktop"]
```

The key file decodes `\n` before the tokenizer reads `Exec`, and the tokenizer splits on the newline, as GLib does.

```entry locale=C desktop=niri
[Desktop Entry]
Type=Application
Name=Lines
Exec=app a\nb
```

```parsed
name: Lines
unlocalized name: Lines
generic name: none
keywords: none
icon: none
argv: ["app", "a", "b"]
terminal: false
try-exec: none
```

Key and group names that GLib rejects read as written. `Name [de]` and `X-Foo]` are keys that nothing reads, and `[]` is a group.

```entry locale=C desktop=niri
[Desktop Entry]
Type=Application
Name=App
Name [de]=x
X-Foo]=x
Exec=app

[]
A=b
```

```parsed
name: App
unlocalized name: App
generic name: none
keywords: none
icon: none
argv: ["app"]
terminal: false
try-exec: none
```

An `Encoding` other than `UTF-8` is a key that nothing reads, so the app shows, where GLib fails the whole file.

```entry locale=C desktop=niri
[Desktop Entry]
Encoding=Legacy-Mixed
Type=Application
Name=Legacy
Exec=app
```

```parsed
name: Legacy
unlocalized name: Legacy
generic name: none
keywords: none
icon: none
argv: ["app"]
terminal: false
try-exec: none
```

`niri` comes first among the current desktops and is in `NotShowIn`, so the entry hides, although `GNOME` is in `OnlyShowIn`.

```entry locale=C desktop=niri:GNOME
[Desktop Entry]
Type=Application
Name=Files
Exec=nautilus
OnlyShowIn=GNOME;
NotShowIn=niri;
```

```parsed
hidden: NotShownIn
```

A boolean is `true` or `1` after trailing ASCII whitespace, which is what GLib trims. A trailing no-break space leaves `NoDisplay` unreadable, so GLib and lyncast both show the entry.

```entry locale=C desktop=niri
[Desktop Entry]
Type=Application
Name=Spaces
Exec=spaces
NoDisplay=true\u{A0}
```

```parsed
name: Spaces
unlocalized name: Spaces
generic name: none
keywords: none
icon: none
argv: ["spaces"]
terminal: false
try-exec: none
```

```entry locale=C desktop=niri
[Desktop Entry]
Hidden=true
```

```parsed
hidden: Hidden
```

```entry locale=C desktop=niri
[Desktop Entry]
Type=Link
Name=Website
URL=https://example.com
```

```parsed
hidden: NotApplication
```

`OnlyShowIn` compares desktop names case-sensitively, so `gnome` does not match `GNOME`.

```entry locale=C desktop=GNOME
[Desktop Entry]
Type=Application
Name=Files
Exec=nautilus
OnlyShowIn=gnome;
```

```parsed
hidden: NotShownIn
```

```entry locale=C desktop=niri
[Desktop Entry]
Type=Application
Exec=nautilus
```

```parsed
hidden: NoName
```

```entry locale=C desktop=niri
[Desktop Entry]
Type=Application
Name=Files
```

```parsed
hidden: NoExec
```

```entry locale=C desktop=niri
[Desktop Entry]
Type=Application
Name=Files
Exec=nautilus "--unterminated
```

```parsed
hidden: BadExec(UnterminatedQuote)
```

## The Exec tokenizer quotes the way GLib does

The Exec tokenizer turns an `Exec` value into the argv that step 8 runs. It reads the value after key-file unescaping, so a file's `\\` reaches it as `\`.

The spec allows only double quotes. GLib expands the field codes and then splits the command line with `g_shell_parse_argv`, as a shell would, so real files also use single quotes and backslashes outside quotes. Wine writes `Start\ Menu` for a path with a space. lyncast splits as `g_shell_parse_argv` does:

- Arguments split on unquoted spaces, tabs, and newlines, and a run of them makes no empty argument.
- Double quotes group text, also inside a longer argument such as `--name="My App"`. Inside them, a backslash escapes `"`, `` ` ``, `$`, `\`, and a newline, and stays before any other character.
- Single quotes group text with no escapes.
- Outside quotes, a backslash escapes the next character, and a backslash before a newline joins the two lines. A backslash at the end of the value is an error.
- Outside quotes, a `#` that starts the value or follows a space or a newline starts a comment, which runs to the next newline. A `#` after a tab, a quote, or other text is text. GLib fails a line that ends with a comment's `#`, and lyncast does too.
- `""` is an empty argument.

Field codes expand inside and outside quotes, before the split, so a code that expands to nothing leaves the `#` after it where a comment can start. A code inside a comment goes with it, even an unknown one, so a `%` just before the comment's newline takes that newline as its code and the comment runs on. A launcher passes no files, so `%f`, `%F`, `%u`, and `%U` expand to nothing, and an argument made only of them disappears. `%c` is the translated Name, `%k` is the desktop file's path, and `%%` is `%`. The deprecated `%d`, `%D`, `%n`, `%N`, `%v`, and `%m` expand to nothing. A standalone `%i` becomes `--icon` and the Icon, or nothing without an Icon. Any other code, `%i` inside a longer argument, a `%` at the end, an unclosed quote, a backslash at the end, or an argv with nothing left is an error, and the entry or action does not show. An unknown code, a `%` at the end, and `%i` inside a longer argument are stricter than GLib, whose `g_app_info_launch` drops an unknown code, keeps a `%` at the end, and runs `app --x=%i` with `--x=--icon` and `firefox` (measured, 2026-10-16).

`g_app_info_launch` also differs from lyncast where the spec defines nothing. These are the known differences, measured on GLib 2.82.2 on the dev box (2026-10-16):

- A backslash before a field code. GLib expands the codes in the raw value and then splits, so the backslash escapes whatever the code leaves after it. `app \%U x` runs `app` with the one argument ` x`, and `app a\%%b` runs `app` with `a%b`. lyncast reads `\%` as a plain `%`, so it gives `["app", "%U", "x"]` and rejects `a\%%b` as `UnknownCode('b')`.
- A code inside double quotes. GLib quotes `%c`, `%k`, and the Icon for a shell before it splits, so the quotes stay inside double quotes, and `"--name=%c"` runs as `--name='Firefox'`. lyncast gives `--name=Firefox`.
- A code inside single quotes whose value holds whitespace or a quote. GLib's shell quoting closes the single quote, so the value splits at its whitespace, and a quote in it leaves a quote that never closes. With Name `Web Browser`, `sh -c 'exec app --name=%c' x` runs `sh` with `-c`, `exec app --name=Web`, `Browser`, and `x`, so `app` gets `--name=Web`. With Name `Bob's`, `app '--name=%c'` fails to launch. lyncast gives `["sh", "-c", "exec app --name=Web Browser", "x"]` and `["app", "--name=Bob's"]`.

The spec leaves a code inside a quoted argument undefined, and lyncast keeps the code's value whole. The `exec` lines below expand with Name `Firefox`, which holds no space, so this entry pins the single-quote case.

```entry locale=C desktop=niri
[Desktop Entry]
Type=Application
Name=Web Browser
Exec=sh -c 'exec app --name=%c' x
```

```parsed
name: Web Browser
unlocalized name: Web Browser
generic name: none
keywords: none
icon: none
argv: ["sh", "-c", "exec app --name=Web Browser", "x"]
terminal: false
try-exec: none
```

Each line expands with Name `Firefox`, Icon `firefox`, and path `/usr/share/applications/firefox.desktop`. The argv is written as Rust prints a `Vec<String>`, and an error as Rust prints the `ExecError`. Each line without a field-code error gives what `g_shell_parse_argv` in GLib 2.82.2 gives for its value with the codes expanded this way, the argv or a failure (measured on the dev box, 2026-10-16).

```exec
firefox %u => ["firefox"]
sh -c "echo a b" => ["sh", "-c", "echo a b"]
sh -c 'echo a b' => ["sh", "-c", "echo a b"]
"/opt/My App/app" --name=%c %k => ["/opt/My App/app", "--name=Firefox", "/usr/share/applications/firefox.desktop"]
app --name="My App" => ["app", "--name=My App"]
app %i --x => ["app", "--icon", "firefox", "--x"]
app 100%% => ["app", "100%"]
app "say \"hi\" \$HOME \n" => ["app", "say \"hi\" $HOME \\n"]
app '\$HOME' => ["app", "\\$HOME"]
app "" x => ["app", "", "x"]
app   x => ["app", "x"]
app\u{9}x => ["app", "x"]
app "--name=%c" => ["app", "--name=Firefox"]
app "a\\b \`c\`" => ["app", "a\\b `c`"]
app %d %D %n %N %v %m x => ["app", "x"]
wine C:\\Program\ Files\\app.lnk => ["wine", "C:\\Program Files\\app.lnk"]
env WINEPREFIX="/home/u/.wine" wine x.exe => ["env", "WINEPREFIX=/home/u/.wine", "wine", "x.exe"]
flatpak run --command=brave com.brave.Browser @@u %U @@ => ["flatpak", "run", "--command=brave", "com.brave.Browser", "@@u", "@@"]
app %f => ["app"]
app %F x => ["app", "x"]
app "%F" x => ["app", "", "x"]
app --x #y => ["app", "--x"]
app\u{9}#y => ["app", "#y"]
app "x"#y => ["app", "x#y"]
app a#b => ["app", "a#b"]
app %U#y => ["app"]
app a\u{A}b => ["app", "a", "b"]
app #c\u{A}next x => ["app", "next", "x"]
app #%x y => ["app"]
app --x #100%\u{A}--y => ["app", "--x"]
app # %\u{A}x => ["app"]
app a\ #x y => ["app", "a "]
# %\u{A}x => error: Empty
app a\\u{A}b => ["app", "ab"]
app "a\\u{A}b" => ["app", "a\nb"]
app --x=%i => error: EmbeddedIcon
app %x => error: UnknownCode('x')
app 100% => error: LonePercent
app "open => error: UnterminatedQuote
app 'open => error: UnterminatedQuote
app # => error: UnterminatedQuote
app #%U => error: UnterminatedQuote
%U => error: Empty
#app x => error: Empty
app foo\ => error: TrailingBackslash
```

```exec no-icon
app %i => ["app"]
```

## lyncast query prints one JSON object per row

Until step 5, `lyncast query <text> --json` scans the apps and ranks them in the same process, with no visits and the system clock. Step 5 moves the work behind the socket, and the expected output below becomes its regression check. `--json` is required until a plain view exists. The query may start with a hyphen.

Each row is one compact JSON object on its own line, with its fields in this order:

| Field | Value |
|---|---|
| `key` | the row's key, such as `app:firefox.desktop` or `cmd:quit` |
| `index` | the flat selection index across sections |
| `kind` | `app`, `command`, `clip`, or `calculator` |
| `section` | `answer`, `results`, `suggestions`, or `commands` |
| `title` | the row's title |
| `actions` | the Actions menu in order, each with its `verb`, its `label`, and the `chords` that run it |

Verbs are `launch`, `launch-action:<id>`, `copy-path`, `copy`, `pin`, `unpin`, `delete`, and `run`. Chords are `enter` and `ctrl-enter`. Both chords run the default item in the MVP. The binary owns these spellings, in `view.rs`, and serializes with `serde_json`. `lyncast-core` has no serde.

The test writes each `desktop` and `executable` block below under a fresh directory, `$TREE`, in `CARGO_TARGET_TMPDIR`. It makes each `symlink` block a symlink to its target as written, and each `fifo` block a FIFO. An `executable` file gets mode 0755, and a `desktop` file keeps 0644, so `bin/firefox-aurora` has no execute bit. A `desktop` block may give `mode=` and an octal mode after its path. It runs the real binary with only the `env` block's variables, unset by any `env -u NAME` and then overridden by any `NAME=value` words before `lyncast` on the command line, and with `$TREE` as its working directory. It compares stdout byte for byte with each `cli` block, and it expects empty stderr and exit 0. A run over 10 s fails its block and ends the test, so a scan that blocks cannot hang it. The table lists every `desktop`, `executable`, `symlink`, and `fifo` block, and each `executable` is a program that some `desktop` block's `Exec` or `TryExec` names.

```env
XDG_DATA_HOME=$TREE/home
XDG_DATA_DIRS=relative:$TREE/local:$TREE/usr:$TREE/flatpak
LANGUAGE=
LANG=de_DE.UTF-8
XDG_CURRENT_DESKTOP=niri
PATH=$TREE/bin
```

The empty `LANGUAGE` counts as unset, so `LANG` decides. `$TREE/local` does not exist, and the walk skips it. Each file proves one rule.

| File | Proves |
|---|---|
| `usr/applications/firefox.desktop` | two Desktop Actions, one with a French name and one with a German name, and TryExec found on `PATH` |
| `bin/firefox` | the program that Firefox's Exec and TryExec name, which seven other entries run too, and the file that Firefox Relative Try's `TryExec` and Firefox Relative Exec's Exec program name by the relative path `bin/firefox` |
| `usr/applications/mozilla/firefox-developer-edition.desktop` | a second Firefox app, whose subdirectory gives the ID `mozilla-firefox-developer-edition.desktop` |
| `bin/firefox-developer-edition` | Firefox Developer Edition's Exec program |
| `usr/applications/org.gnome.Epiphany.desktop` | a keyword-only match, which shows and ranks last |
| `bin/epiphany` | Web's Exec program |
| `usr/applications/firefox-crash-reporter.desktop` | NoDisplay hides |
| `bin/crashreporter` | the Crash Reporter's Exec program, so only `NoDisplay` hides that entry |
| `home/applications/firefox-esr.desktop` | a `Hidden=true` file in an earlier directory masks `usr/applications/firefox-esr.desktop` |
| `usr/applications/firefox-esr.desktop` | the system entry that the `Hidden=true` file masks |
| `bin/firefox-esr` | the system Firefox ESR's Exec program, so only the `Hidden=true` file hides that entry |
| `home/applications/firefox-kiosk.desktop` | a file that does not parse, because of its last line, still masks `usr/applications/firefox-kiosk.desktop` |
| `usr/applications/firefox-kiosk.desktop` | the system entry that the unparseable file masks |
| `usr/applications/firefox-plasma.desktop` | `OnlyShowIn=KDE` hides on niri |
| `bin/firefox-plasma` | the Plasma entry's Exec program, so only `OnlyShowIn` hides that entry |
| `usr/applications/firefox-nightly.desktop` | a TryExec missing from `PATH` hides |
| `usr/applications/firefox-flatpak.desktop` | an Exec program missing from `PATH` hides |
| `usr/applications/firefox-aurora.desktop` | a TryExec on `PATH` without the execute bit hides |
| `bin/firefox-aurora` | the Aurora TryExec program, with no execute bit |
| `usr/applications/firefox-snap.desktop` | an absolute TryExec that does not exist hides |
| `usr/applications/firefox-gnome.desktop` | `NotShowIn=niri;` hides on niri, so the binary reads `XDG_CURRENT_DESKTOP` |
| `relative/applications/firefox-beta.desktop` | a relative `XDG_DATA_DIRS` entry is ignored, although the working directory holds it |
| `bin/firefox-beta` | Firefox Beta's Exec program, so only the relative directory hides that entry |
| `home/applications/htop.desktop` | the earliest directory wins over `usr/applications/htop.desktop`, and a terminal app shows |
| `usr/applications/htop.desktop` | the system entry that the user's htop wins over |
| `bin/htop` | the program both htop entries run |
| `home/applications/notes.desktop` | a byte that is not UTF-8, in a translation lyncast never reads, does not hide the user's entry, which masks `usr/applications/notes.desktop` |
| `usr/applications/notes.desktop` | the system entry that the user's Notes wins over |
| `bin/notes` | the program that the Notes entries, the masked system entries, Masked TryExec Mine, Masked Inside, Symlinked, Sh Check, and Zzz App run |
| `home/applications/flatpak` | a symlink to the later root `$TREE/flatpak`, which the walk follows, and which does not stop that root's own walk |
| `flatpak/applications/org.gnome.Terminal.desktop` | an app that shows once under each path, as `flatpak-org.gnome.Terminal.desktop` and `org.gnome.Terminal.desktop` |
| `bin/gnome-terminal` | the program that the Terminal entries run |
| `flatpak/applications/loop` | a symlink to its own directory, a cycle the walk does not follow |
| `usr/applications/terminal-twin.desktop` | a file whose ID the nested file takes, although the test writes this one first, so a walk in the directory's own order would read it first on the dev box's XFS |
| `usr/applications/terminal/twin.desktop` | the walk reads names in order, so `terminal/` comes before `terminal-twin.desktop` and its file takes the ID `terminal-twin.desktop` |
| `usr/applications/pathless-shell.desktop` | an Exec program, `sh`, that only the search path for an unset `PATH` finds |
| `usr/applications/pathless-here.desktop` | an Exec program, `here-tool`, that only the working directory holds |
| `here-tool` | the program that Pathless Here, Workdir Here, and Workdir Dot run, in the working directory |
| `home/applications/masked-device.desktop` | a symlink to `/dev/zero`, which the scan does not read, still masks `usr/applications/masked-device.desktop` |
| `usr/applications/masked-device.desktop` | the system entry that the device link masks |
| `home/applications/masked-link.desktop` | a broken symlink still masks `usr/applications/masked-link.desktop` |
| `usr/applications/masked-link.desktop` | the system entry that the broken link masks |
| `home/applications/masked-folder.desktop/inside.desktop` | a file in a directory with a `.desktop` name, which the walk does not enter, while the directory masks `usr/applications/masked-folder.desktop` |
| `usr/applications/masked-folder.desktop` | the system entry that the directory masks |
| `home/applications/masked-program.desktop` | a user entry whose Exec program is missing hides, and it masks `usr/applications/masked-program.desktop` |
| `usr/applications/masked-program.desktop` | the system entry that the user's entry masks |
| `home/applications/masked-fifo.desktop` | a FIFO, which the scan does not open, still masks `usr/applications/masked-fifo.desktop` |
| `usr/applications/masked-fifo.desktop` | the system entry that the FIFO masks |
| `home/applications/masked-tryexec.desktop` | a user entry whose `TryExec` is missing hides, and it masks `usr/applications/masked-tryexec.desktop` |
| `usr/applications/masked-tryexec.desktop` | the system entry that the user's entry masks |
| `home/applications/masked-unreadable.desktop` | a file with mode 000, which this user cannot read, still masks `usr/applications/masked-unreadable.desktop`. Root can read it, and its `Hidden=true` masks the ID then |
| `usr/applications/masked-unreadable.desktop` | the system entry that the unreadable file masks |
| `home/applications/symlinked.desktop` | a symlink to a desktop file outside every `applications` directory, which the scan reads, so Symlinked shows |
| `outside/symlinked.desktop` | the file the link points to, which the walk never reaches itself |
| `usr/applications/sh-check.desktop` | an absolute TryExec that exists, `/bin/sh`, shows |
| `usr/applications/workdir-shell.desktop` | a bare `TryExec` and Exec program, `sh`, that only the entry's `Path` directory, `/bin`, holds |
| `usr/applications/workdir-here.desktop` | a bare Exec program, `here-tool`, that only the working directory holds, which a relative `PATH` entry does not find when `Path` is set |
| `usr/applications/workdir-dot.desktop` | a relative `Path`, `.`, which the binary does not search first, and under which a relative `PATH` entry finds `here-tool` |
| `usr/applications/a-link` | a symlink to `zz` in the same root, which the walk reaches first, because `a-link` sorts before `zz` |
| `usr/applications/zz/zz-app.desktop` | an app that shows once under each path, as `a-link-zz-app.desktop` and `zz-zz-app.desktop`, so the walk leaves `zz` when it returns from the link |
| `usr/applications/firefox-folder-try.desktop` | a `TryExec` that is a directory, `/usr`, is never found, so the entry hides |
| `usr/applications/firefox-folder-exec.desktop` | an Exec program that is a directory, `/usr`, is never found, so the entry hides |
| `usr/applications/firefox-relative-try.desktop` | a relative `TryExec` with a `/`, `bin/firefox`, is never found, although the working directory holds it, so the entry hides |
| `usr/applications/firefox-relative-exec.desktop` | a relative Exec program with a `/`, `bin/firefox`, is never found, although the working directory holds it, so the entry hides |

```desktop usr/applications/firefox.desktop
[Desktop Entry]
Type=Application
Name=Firefox
GenericName=Web Browser
GenericName[de]=Webbrowser
Keywords=Internet;WWW;Browser;Web;Explorer;
Exec=firefox %u
TryExec=firefox
Icon=firefox
Actions=new-window;new-private-window;

[Desktop Action new-window]
Name=New Window
Name[fr]=Nouvelle fenêtre
Exec=firefox --new-window %u

[Desktop Action new-private-window]
Name=New Private Window
Name[de]=Neues privates Fenster
Exec=firefox --private-window %u
```

```executable bin/firefox
#!/bin/sh
```

```desktop usr/applications/mozilla/firefox-developer-edition.desktop
[Desktop Entry]
Type=Application
Name=Firefox Developer Edition
GenericName=Web Browser
Exec=firefox-developer-edition %u
```

```executable bin/firefox-developer-edition
#!/bin/sh
```

```desktop usr/applications/org.gnome.Epiphany.desktop
[Desktop Entry]
Type=Application
Name=Web
GenericName=Web Browser
Keywords=web;browser;internet;firefox;
Exec=epiphany %U
```

```executable bin/epiphany
#!/bin/sh
```

```desktop usr/applications/firefox-crash-reporter.desktop
[Desktop Entry]
Type=Application
Name=Firefox Crash Reporter
Exec=crashreporter
NoDisplay=true
```

```executable bin/crashreporter
#!/bin/sh
```

```desktop home/applications/firefox-esr.desktop
[Desktop Entry]
Hidden=true
```

```desktop usr/applications/firefox-esr.desktop
[Desktop Entry]
Type=Application
Name=Firefox ESR
Exec=firefox-esr %u
```

```executable bin/firefox-esr
#!/bin/sh
```

```desktop home/applications/firefox-kiosk.desktop
[Desktop Entry]
Type=Application
Name=Firefox Kiosk Mode
Exec=firefox --kiosk %u
this is not a key
```

```desktop usr/applications/firefox-kiosk.desktop
[Desktop Entry]
Type=Application
Name=Firefox Kiosk
Exec=firefox --kiosk %u
```

```desktop usr/applications/firefox-plasma.desktop
[Desktop Entry]
Type=Application
Name=Firefox Plasma Integration
Exec=firefox-plasma
OnlyShowIn=KDE;
```

```executable bin/firefox-plasma
#!/bin/sh
```

```desktop usr/applications/firefox-nightly.desktop
[Desktop Entry]
Type=Application
Name=Firefox Nightly
Exec=firefox -P nightly %u
TryExec=firefox-nightly
```

```desktop usr/applications/firefox-flatpak.desktop
[Desktop Entry]
Type=Application
Name=Firefox Flatpak
Exec=flatpak run org.mozilla.firefox %u
```

```desktop usr/applications/firefox-aurora.desktop
[Desktop Entry]
Type=Application
Name=Firefox Aurora
Exec=firefox -P aurora %u
TryExec=firefox-aurora
```

```desktop bin/firefox-aurora
#!/bin/sh
```

```desktop usr/applications/firefox-snap.desktop
[Desktop Entry]
Type=Application
Name=Firefox Snap
Exec=firefox %u
TryExec=/nonexistent/snap/bin/firefox
```

```desktop usr/applications/firefox-gnome.desktop
[Desktop Entry]
Type=Application
Name=Firefox GNOME Integration
Exec=firefox %u
NotShowIn=niri;
```

```desktop relative/applications/firefox-beta.desktop
[Desktop Entry]
Type=Application
Name=Firefox Beta
Exec=firefox-beta %u
```

```executable bin/firefox-beta
#!/bin/sh
```

```desktop home/applications/htop.desktop
[Desktop Entry]
Type=Application
Name=htop
Exec=htop
Terminal=true
```

```executable bin/htop
#!/bin/sh
```

```desktop usr/applications/htop.desktop
[Desktop Entry]
Type=Application
Name=Htop System Monitor
Exec=htop
Terminal=true
```

```desktop home/applications/notes.desktop
[Desktop Entry]
Type=Application
Name=Notes
Comment[fr]=Bloc-notes caf\x{E9}
Exec=notes --mine
```

```desktop usr/applications/notes.desktop
[Desktop Entry]
Type=Application
Name=Notes System
Exec=notes
```

```executable bin/notes
#!/bin/sh
```

```symlink home/applications/flatpak -> ../../flatpak/applications
```

```desktop flatpak/applications/org.gnome.Terminal.desktop
[Desktop Entry]
Type=Application
Name=Terminal
Exec=gnome-terminal
```

```executable bin/gnome-terminal
#!/bin/sh
```

```symlink flatpak/applications/loop -> .
```

```desktop usr/applications/terminal-twin.desktop
[Desktop Entry]
Type=Application
Name=Terminal Flat
Exec=gnome-terminal
```

```desktop usr/applications/terminal/twin.desktop
[Desktop Entry]
Type=Application
Name=Terminal Nested
Exec=gnome-terminal
```

```desktop usr/applications/pathless-shell.desktop
[Desktop Entry]
Type=Application
Name=Pathless Shell
Exec=sh -c true
```

```desktop usr/applications/pathless-here.desktop
[Desktop Entry]
Type=Application
Name=Pathless Here
Exec=here-tool
```

```executable here-tool
#!/bin/sh
```

```symlink home/applications/masked-device.desktop -> /dev/zero
```

```desktop usr/applications/masked-device.desktop
[Desktop Entry]
Type=Application
Name=Masked Device
Exec=notes
```

```symlink home/applications/masked-link.desktop -> nowhere.desktop
```

```desktop usr/applications/masked-link.desktop
[Desktop Entry]
Type=Application
Name=Masked Link
Exec=notes
```

```desktop home/applications/masked-folder.desktop/inside.desktop
[Desktop Entry]
Type=Application
Name=Masked Inside
Exec=notes
```

```desktop usr/applications/masked-folder.desktop
[Desktop Entry]
Type=Application
Name=Masked Folder
Exec=notes
```

```desktop home/applications/masked-program.desktop
[Desktop Entry]
Type=Application
Name=Masked Program Mine
Exec=ghost-program
```

```desktop usr/applications/masked-program.desktop
[Desktop Entry]
Type=Application
Name=Masked Program
Exec=notes
```

```fifo home/applications/masked-fifo.desktop
```

```desktop usr/applications/masked-fifo.desktop
[Desktop Entry]
Type=Application
Name=Masked Fifo
Exec=notes
```

```desktop home/applications/masked-tryexec.desktop
[Desktop Entry]
Type=Application
Name=Masked TryExec Mine
Exec=notes
TryExec=ghost-try
```

```desktop usr/applications/masked-tryexec.desktop
[Desktop Entry]
Type=Application
Name=Masked TryExec
Exec=notes
```

```desktop home/applications/masked-unreadable.desktop mode=000
Hidden=true
```

```desktop usr/applications/masked-unreadable.desktop
[Desktop Entry]
Type=Application
Name=Masked Unreadable
Exec=notes
```

```symlink home/applications/symlinked.desktop -> ../../outside/symlinked.desktop
```

```desktop outside/symlinked.desktop
[Desktop Entry]
Type=Application
Name=Symlinked
Exec=notes
```

```desktop usr/applications/sh-check.desktop
[Desktop Entry]
Type=Application
Name=Sh Check
Exec=notes --check
TryExec=/bin/sh
```

```desktop usr/applications/workdir-shell.desktop
[Desktop Entry]
Type=Application
Name=Workdir Shell
Path=/bin
Exec=sh
TryExec=sh
```

```desktop usr/applications/workdir-here.desktop
[Desktop Entry]
Type=Application
Name=Workdir Here
Path=/bin
Exec=here-tool
```

```desktop usr/applications/workdir-dot.desktop
[Desktop Entry]
Type=Application
Name=Workdir Dot
Path=.
Exec=here-tool
```

```symlink usr/applications/a-link -> zz
```

```desktop usr/applications/zz/zz-app.desktop
[Desktop Entry]
Type=Application
Name=Zzz App
Exec=notes
```

```desktop usr/applications/firefox-folder-try.desktop
[Desktop Entry]
Type=Application
Name=Firefox Folder Try
Exec=firefox %u
TryExec=/usr
```

```desktop usr/applications/firefox-folder-exec.desktop
[Desktop Entry]
Type=Application
Name=Firefox Folder Exec
Exec=/usr %u
```

```desktop usr/applications/firefox-relative-try.desktop
[Desktop Entry]
Type=Application
Name=Firefox Relative Try
Exec=firefox %u
TryExec=bin/firefox
```

```desktop usr/applications/firefox-relative-exec.desktop
[Desktop Entry]
Type=Application
Name=Firefox Relative Exec
Exec=bin/firefox %u
```

Firefox and Firefox Developer Edition both score 12 on their titles and tie through rule 13, so the name decides. Web shows only through its keyword `firefox`, so it has no rule-9 score and comes last. No command aligns with `firef`.

```cli
$ lyncast query firef --json
{"key":"app:firefox.desktop","index":0,"kind":"app","section":"results","title":"Firefox","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"launch-action:new-window","label":"New Window","chords":[]},{"verb":"launch-action:new-private-window","label":"Neues privates Fenster","chords":[]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
{"key":"app:mozilla-firefox-developer-edition.desktop","index":1,"kind":"app","section":"results","title":"Firefox Developer Edition","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
{"key":"app:org.gnome.Epiphany.desktop","index":2,"kind":"app","section":"results","title":"Web","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
```

Apps and commands rank in one list. Open Config scores 4 and comes before Web, whose only match is a keyword.

```cli
$ lyncast query fi --json
{"key":"app:firefox.desktop","index":0,"kind":"app","section":"results","title":"Firefox","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"launch-action:new-window","label":"New Window","chords":[]},{"verb":"launch-action:new-private-window","label":"Neues privates Fenster","chords":[]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
{"key":"app:mozilla-firefox-developer-edition.desktop","index":1,"kind":"app","section":"results","title":"Firefox Developer Edition","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
{"key":"cmd:open-config","index":2,"kind":"command","section":"results","title":"Open Config","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"app:org.gnome.Epiphany.desktop","index":3,"kind":"app","section":"results","title":"Web","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
```

A command line may set variables before `lyncast`, and they override the `env` block. `LC_ALL` comes before `LANG`, so in `C` the private-window action keeps its untranslated name. Firefox meets rule 3, and Web again shows only through its keyword.

```cli
$ LC_ALL=C lyncast query firefox --json
{"key":"app:firefox.desktop","index":0,"kind":"app","section":"results","title":"Firefox","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"launch-action:new-window","label":"New Window","chords":[]},{"verb":"launch-action:new-private-window","label":"New Private Window","chords":[]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
{"key":"app:mozilla-firefox-developer-edition.desktop","index":1,"kind":"app","section":"results","title":"Firefox Developer Edition","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
{"key":"app:org.gnome.Epiphany.desktop","index":2,"kind":"app","section":"results","title":"Web","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
```

`LC_ALL` comes before `LC_MESSAGES`, so in `C` the private-window action keeps its untranslated name again.

```cli
$ LC_ALL=C LC_MESSAGES=de_DE.UTF-8 lyncast query firefox --json
{"key":"app:firefox.desktop","index":0,"kind":"app","section":"results","title":"Firefox","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"launch-action:new-window","label":"New Window","chords":[]},{"verb":"launch-action:new-private-window","label":"New Private Window","chords":[]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
{"key":"app:mozilla-firefox-developer-edition.desktop","index":1,"kind":"app","section":"results","title":"Firefox Developer Edition","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
{"key":"app:org.gnome.Epiphany.desktop","index":2,"kind":"app","section":"results","title":"Web","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
```

`LC_MESSAGES` comes before `LANG`.

```cli
$ LC_MESSAGES=de_DE.UTF-8 LANG=C lyncast query firefox --json
{"key":"app:firefox.desktop","index":0,"kind":"app","section":"results","title":"Firefox","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"launch-action:new-window","label":"New Window","chords":[]},{"verb":"launch-action:new-private-window","label":"Neues privates Fenster","chords":[]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
{"key":"app:mozilla-firefox-developer-edition.desktop","index":1,"kind":"app","section":"results","title":"Firefox Developer Edition","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
{"key":"app:org.gnome.Epiphany.desktop","index":2,"kind":"app","section":"results","title":"Web","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
```

`LANGUAGE` comes before `LC_ALL`, as GLib reads it, and it lists locale names to try in order. With `fr:de` the new-window action has a French name, and the private-window action, which has none, falls back to German.

```cli
$ LANGUAGE=fr:de LC_ALL=C lyncast query firefox --json
{"key":"app:firefox.desktop","index":0,"kind":"app","section":"results","title":"Firefox","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"launch-action:new-window","label":"Nouvelle fenêtre","chords":[]},{"verb":"launch-action:new-private-window","label":"Neues privates Fenster","chords":[]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
{"key":"app:mozilla-firefox-developer-edition.desktop","index":1,"kind":"app","section":"results","title":"Firefox Developer Edition","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
{"key":"app:org.gnome.Epiphany.desktop","index":2,"kind":"app","section":"results","title":"Web","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
```

The list replaces the locale. The `env` block's `LANG` is German, but with `LANGUAGE=fr` the private-window action keeps its untranslated name.

```cli
$ LANGUAGE=fr lyncast query firefox --json
{"key":"app:firefox.desktop","index":0,"kind":"app","section":"results","title":"Firefox","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"launch-action:new-window","label":"Nouvelle fenêtre","chords":[]},{"verb":"launch-action:new-private-window","label":"New Private Window","chords":[]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
{"key":"app:mozilla-firefox-developer-edition.desktop","index":1,"kind":"app","section":"results","title":"Firefox Developer Edition","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
{"key":"app:org.gnome.Epiphany.desktop","index":2,"kind":"app","section":"results","title":"Web","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
```

The empty query is a fresh install. Suggestions holds Clipboard History and the first 4 apps in name order, and Commands holds the other 8 commands. The other nine apps that show, the two Terminals, Terminal Nested, Sh Check, Symlinked, Web, Workdir Shell, and the two Zzz Apps, are in neither section. `index` counts across both sections.

```cli
$ lyncast query "" --json
{"key":"cmd:clipboard-history","index":0,"kind":"command","section":"suggestions","title":"Clipboard History","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"app:firefox.desktop","index":1,"kind":"app","section":"suggestions","title":"Firefox","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"launch-action:new-window","label":"New Window","chords":[]},{"verb":"launch-action:new-private-window","label":"Neues privates Fenster","chords":[]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
{"key":"app:mozilla-firefox-developer-edition.desktop","index":2,"kind":"app","section":"suggestions","title":"Firefox Developer Edition","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
{"key":"app:htop.desktop","index":3,"kind":"app","section":"suggestions","title":"htop","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
{"key":"app:notes.desktop","index":4,"kind":"app","section":"suggestions","title":"Notes","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
{"key":"cmd:lock-screen","index":5,"kind":"command","section":"commands","title":"Lock Screen","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:log-out","index":6,"kind":"command","section":"commands","title":"Log Out","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:open-config","index":7,"kind":"command","section":"commands","title":"Open Config","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:power-off","index":8,"kind":"command","section":"commands","title":"Power Off","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:quit","index":9,"kind":"command","section":"commands","title":"Quit lyncast","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:reboot","index":10,"kind":"command","section":"commands","title":"Reboot","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:reload-apps","index":11,"kind":"command","section":"commands","title":"Reload Apps","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
{"key":"cmd:suspend","index":12,"kind":"command","section":"commands","title":"Suspend","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
```

The user's Notes holds a byte that is not UTF-8 in its French comment. The entry still shows, and the system file with its ID is never read.

```cli
$ lyncast query notes --json
{"key":"app:notes.desktop","index":0,"kind":"app","section":"results","title":"Notes","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
```

The user's `flatpak` link reaches `$TREE/flatpak/applications`, so Terminal shows under the link's ID and then under its own, when the walk reaches that root. The two rows tie until the name, and the keys decide. The `loop` link leads back to a directory the walk is already in, so it adds no ID. Terminal Nested holds the ID `terminal-twin.desktop`, and Terminal Flat, which comes later in the walk, does not show. All three titles score 10 and start with `term`, so the name orders them.

```cli
$ lyncast query term --json
{"key":"app:flatpak-org.gnome.Terminal.desktop","index":0,"kind":"app","section":"results","title":"Terminal","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
{"key":"app:org.gnome.Terminal.desktop","index":1,"kind":"app","section":"results","title":"Terminal","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
{"key":"app:terminal-twin.desktop","index":2,"kind":"app","section":"results","title":"Terminal Nested","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
```

The `a-link` link leads to `zz` in the same root, and the walk reaches it first, because `a-link` sorts before `zz`. The walk leaves `zz` when it returns from the link, so it walks `zz` again under its own path, and Zzz App shows under both IDs, as GLib 2.82.2's `g_app_info_get_all` lists them (measured on the dev box, 2026-10-16). Both titles score 8 and start with `zzz`, so the rows tie until the name, and the keys decide. No command aligns with `zzz`.

```cli
$ lyncast query zzz --json
{"key":"app:a-link-zz-app.desktop","index":0,"kind":"app","section":"results","title":"Zzz App","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
{"key":"app:zz-zz-app.desktop","index":1,"kind":"app","section":"results","title":"Zzz App","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
```

A relative `XDG_DATA_HOME` is ignored, as the XDG Base Directory spec asks, although the working directory holds `home/applications`. With no `HOME` either, the walk starts at `$XDG_DATA_DIRS`, and the system's Notes shows.

```cli
$ XDG_DATA_HOME=home lyncast query notes --json
{"key":"app:notes.desktop","index":0,"kind":"app","section":"results","title":"Notes System","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
```

An `XDG_DATA_DIRS` entry that is not UTF-8 is ignored, as a relative one is, and the binary still reads the others.

```cli
$ XDG_DATA_DIRS=$TREE/nowhere\x{FF}:$TREE/usr lyncast query firef --json
{"key":"app:firefox.desktop","index":0,"kind":"app","section":"results","title":"Firefox","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"launch-action:new-window","label":"New Window","chords":[]},{"verb":"launch-action:new-private-window","label":"Neues privates Fenster","chords":[]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
{"key":"app:mozilla-firefox-developer-edition.desktop","index":1,"kind":"app","section":"results","title":"Firefox Developer Edition","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
{"key":"app:org.gnome.Epiphany.desktop","index":2,"kind":"app","section":"results","title":"Web","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
```

A `PATH` entry that is not UTF-8 hides nothing, and the binary still searches the others for `notes`.

```cli
$ PATH=$TREE/nowhere\x{FF}:$TREE/bin lyncast query notes --json
{"key":"app:notes.desktop","index":0,"kind":"app","section":"results","title":"Notes","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
```

`env -u` unsets a variable. With `PATH` unset, the binary searches `/bin:/usr/bin:.`, as GLib does, so it finds `sh` in `/bin` and `here-tool` in the working directory. Both titles start with the query and score the same, so the name orders them.

```cli
$ env -u PATH lyncast query pathless --json
{"key":"app:pathless-here.desktop","index":0,"kind":"app","section":"results","title":"Pathless Here","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
{"key":"app:pathless-shell.desktop","index":1,"kind":"app","section":"results","title":"Pathless Shell","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
```

An empty `PATH` is not unset. Its one entry is empty. Neither Pathless entry has a `Path` key, so the empty entry names the working directory, as in GLib, and only `here-tool` is found.

```cli
$ PATH= lyncast query pathless --json
{"key":"app:pathless-here.desktop","index":0,"kind":"app","section":"results","title":"Pathless Here","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
```

`env -u` unsets before the `NAME=value` words set, as env(1) orders them, so this line runs with an empty `PATH`.

```cli
$ env -u PATH PATH= lyncast query pathless --json
{"key":"app:pathless-here.desktop","index":0,"kind":"app","section":"results","title":"Pathless Here","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
```

An entry's `Path` key names the directory it runs in. When `Path` is absolute, the binary looks up a bare program there first, as GLib does. So Workdir Shell finds `sh` in `/bin`, although `PATH` holds only `$TREE/bin`. Workdir Here's program is only in the working directory, so it does not show. Workdir Dot's `Path` is `.`, which is relative, so the binary does not look there first, and it does not show either.

```cli
$ lyncast query workdir --json
{"key":"app:workdir-shell.desktop","index":0,"kind":"app","section":"results","title":"Workdir Shell","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
```

When the entry has a `Path`, a relative `PATH` entry, such as the empty one, names a directory under `Path` and not the working directory, as in GLib. So Workdir Here still does not show, although the working directory holds `here-tool`. Workdir Dot finds it in `./`, under its `Path`. Both titles that show score the same, so the name orders them.

```cli
$ PATH= lyncast query workdir --json
{"key":"app:workdir-dot.desktop","index":0,"kind":"app","section":"results","title":"Workdir Dot","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
{"key":"app:workdir-shell.desktop","index":1,"kind":"app","section":"results","title":"Workdir Shell","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
```

Each user file named `masked-*` takes its ID from a system entry, so `masked` finds nothing. `masked-device.desktop` is a link to `/dev/zero`, and `masked-fifo.desktop` is a FIFO. The binary opens neither, because it reads only a regular file of at most 1 MiB. `masked-link.desktop` is a broken link, and `masked-folder.desktop` is a directory, which the walk does not enter. `masked-program.desktop` parses, but its Exec program is missing, and `masked-tryexec.desktop` names a `TryExec` that is missing. This user cannot read `masked-unreadable.desktop`. GLib 2.82.2's `g_app_info_get_all` lists none of these IDs either, once a writer opens the FIFO it blocks on.

```cli
$ lyncast query masked --json
```

The user's `symlinked.desktop` is a symlink to a file outside every `applications` directory. The scan reads a regular file after symlinks, so Symlinked shows, as in GLib.

```cli
$ lyncast query symlinked --json
{"key":"app:symlinked.desktop","index":0,"kind":"app","section":"results","title":"Symlinked","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
```

An absolute `TryExec` is found when it is a file this user may execute, so Sh Check, whose `TryExec` is `/bin/sh`, shows.

```cli
$ lyncast query check --json
{"key":"app:sh-check.desktop","index":0,"kind":"app","section":"results","title":"Sh Check","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
```

A query that starts with a hyphen is a query, not an option, unless it is an option's name or `--`. `-5+3` matches nothing in the tree. The calculator card joins the query in step 5, when the daemon builds rows.

```cli
$ lyncast query -5+3 --json
```

`-h` is a query too, and only the long `--help` prints help for `query`. The needle is `h`. htop's title starts with it and scores 4, and `History` is a word start in Clipboard History that scores 3.

```cli
$ lyncast query -h --json
{"key":"app:htop.desktop","index":0,"kind":"app","section":"results","title":"htop","actions":[{"verb":"launch","label":"Open","chords":["enter","ctrl-enter"]},{"verb":"copy-path","label":"Copy Path to .desktop","chords":[]}]}
{"key":"cmd:clipboard-history","index":1,"kind":"command","section":"results","title":"Clipboard History","actions":[{"verb":"run","label":"Run","chords":["enter","ctrl-enter"]}]}
```

`--json` and `--help` are options, and so is either one with `=` and a value. Such a query needs `--` after `--json`. Here the query `--json` has the needle `json`, which matches nothing.

```cli
$ lyncast query --json -- --json
```

The query `--` itself ends the options, so it needs `--` after `--json` too. Its needle is empty, because `-` is a separator, so it matches nothing.

```cli
$ lyncast query --json -- --
```

## The worst bench keystroke takes 71 µs on 300 apps and 254 µs on 1,000

`cargo run --release -p lyncast-core --example rank_bench` times `Index::rows` per keystroke, the whole path from the query text to the rows. Its index has 300 or 1,000 apps plus the nine commands. The first 216 names are the real ones in `bench/score-map`, and the rest are two-word names from 40 brand words and 40 nouns. Three apps in four have one of 50 generic names, and each has 0 to 6 keywords from 150 words, a few of them common. Every tenth app has visits, half with a search term. The queries are every prefix up to 8 characters of every 37th name, plus a few common names and misses. A long group holds a 64- and a 128-character sentence and a 1,024-byte paste.

Each of 7 runs repeats one loop for about 50 ms. Each iteration times three passes back to back: `Index::rows` over the group, frizbee alone on the bench's own copy of the haystacks, and cloning and dropping the returned rows, which makes the same allocations as building them. The remainder is derived, not timed. It is each iteration's `Index::rows` time minus its other two, averaged over the run. The bench's frizbee pass is not the index's own, so the remainder is biased. Against a probe that timed the index's own pass, it read 8% to 15% low in the 3- and 4-character cells on 300 apps, and 6% to 19% high on 1,000. The bench's pass ran 0.2 to 0.6 µs slower than the index's on 300 apps and 0.2 to 1.9 µs faster on 1,000 (measured, 2026-10-16). Each call to `Index::rows` is also timed on its own inside the loop, and the worst query is the one whose median call is slowest.

On the dev box (AMD EPYC 9R14, pinned to one idle core, load average 7 to 10), as medians of one bench run's 7 runs, measured on 2026-10-16. The remainder column also gives the range of its 7 samples.

| Query | Apps | Rows shown | Mean | Worst query | frizbee | Building rows | Remainder, derived |
|---|---|---|---|---|---|---|---|
| 1 character | 300 | 73 | 39.4 µs | 70.7 µs, `e` | 25.7 µs | 5.2 µs | 8.6 µs, 8.3 to 8.8 |
| 2 characters | 300 | 87 | 40.7 µs | 67.4 µs, `te` | 25.1 µs | 6.1 µs | 9.4 µs, 9.1 to 9.7 |
| 3 characters | 300 | 25 | 25.2 µs | 56.1 µs, `ter` | 20.1 µs | 1.8 µs | 3.3 µs, 3.3 to 3.7 |
| 4 or more | 300 | 7 | 19.7 µs | 39.9 µs, `term` | 17.0 µs | 0.5 µs | 2.3 µs, 2.2 to 2.3 |
| long | 300 | 0 | 1.7 µs | 2.6 µs, the paste | none | 0.0 µs | 1.7 µs, 1.7 to 1.7 |
| empty | 300 | 14 | 2.5 µs | 2.5 µs | none | 0.6 µs | 1.9 µs, 1.9 to 1.9 |
| 1 character | 1,000 | 236 | 149.1 µs | 254.3 µs, `e` | 89.6 µs | 15.6 µs | 44.1 µs, 43.4 to 45.2 |
| 2 characters | 1,000 | 254 | 134.4 µs | 250.6 µs, `te` | 80.0 µs | 16.3 µs | 37.8 µs, 37.5 to 38.5 |
| 3 characters | 1,000 | 78 | 90.2 µs | 210.5 µs, `ter` | 70.9 µs | 5.4 µs | 13.9 µs, 13.6 to 14.5 |
| 4 or more | 1,000 | 26 | 66.2 µs | 139.3 µs, `term` | 55.4 µs | 2.4 µs | 8.4 µs, 8.2 to 8.6 |
| long | 1,000 | 0 | 4.0 µs | 4.9 µs, the paste | none | 0.0 µs | 4.0 µs, 4.0 to 4.0 |
| empty | 1,000 | 14 | 6.6 µs | 6.6 µs | none | 0.6 µs | 6.0 µs, 6.0 to 6.0 |

Three more bench runs on the same core gave every typed group's mean and worst query within 5% of these, except the first run's first group, one character on 300 apps, at 42.3 µs and 78.9 µs. The long and empty rows stayed within 4%. Within this run, the 7 samples of a typed group's mean stayed within 3% of their median.

The joined titles and subtitles raised these costs. They add 444 haystacks on 300 apps and 1,492 on 1,000, about 31% more, and each is longer than the title or the subtitle alone. They also show more rows, since a query can run from the end of a title into its subtitle. Two runs of the same bench without them, on the same core minutes apart, gave worst queries of 52 to 53 µs on 300 apps and 199 to 201 µs on 1,000.

The single frizbee pass is the largest share of every typed query, 60% to 86%. It is linear in the haystacks, at about 9 ns each for queries of 4 or more characters and 13 to 15 ns for one character, when more of them align and frizbee scores them. 300 apps have 1,899 haystacks and 1,000 have 6,175. Building rows grows with the rows shown, and a one-letter query shows about one entry in four. The remainder holds the keys and the sort, which grow with the rows shown. It also holds what every query pays whatever it shows, the fold and `Index::learned`, a pass over every entry and every visit. The long row is close to that cost alone, since it returns before frizbee. `perf` is not installed on the dev box, so no profile backs the split.

A query longer than every field skips frizbee, and the bench's longest field, a joined title and subtitle, has 48 characters on 300 apps and 52 on 1,000. A desktop with a field as long as the needle pays frizbee's matcher setup, which zeroes about 4 KB per needle byte. On one field as long as the needle, a 55-character needle costs 8.2 µs and a 109-character one 28 µs, and an 896-character one about 2.1 ms, 2.04 to 2.08 ms over four runs. In those runs the 109-character needle reused freed pages, and the 896-character one, whose matrices take about 3.6 MB, got fresh pages on every call. A count of minor page faults showed about 890 per call for 896 characters and none for 109. In about one bench process in five, the 896-character needle reuses pages too, and it then costs about 0.61 ms (measured, 2026-10-16). With glibc's mmap threshold fixed at 128 KiB, which the matrices of a needle over about 62 bytes cross, the allocator maps fresh pages for both. Six such runs gave the 109-character needle 28, 130, 131, 225, 226, and 228 µs, and the 896-character one 2.11 to 2.18 ms.

Query text over 1,024 bytes returns before the fold, in 0.02 µs. Before any cap, a 100,000-character paste took 306 to 318 µs on the bench's 300 apps. Under the cap, 1,024 bytes of non-ASCII text that matches nothing took 26 to 28 µs on 300 apps for Cyrillic. The slowest text in two runs, Hangul with final consonants, took 31 µs. On an index with no apps the same texts took 25 to 30 µs, so the fold is most of that cost. With a 4,096-byte cap the same texts took about 100 µs (measured, 2026-10-16).

The fan-out budget is under 0.1 ms per keystroke, and the calculator adds 0.45 µs on its 26 shared inputs ([calculator](calculator.md#the-ports-cost-per-keystroke)). On 300 apps every bench query stays under it, the worst at 71 µs, or 79 µs in the slowest of four bench runs. On 1,000 apps the worst query of every typed group breaks it, from 139 µs at 4 or more characters to 254 µs at one. A paste of about 100 characters can break it on any desktop with a field that long. 109 characters took 228 µs in the worst of six runs, and 896 characters take about 2.1 ms.

Two levers would cut the frizbee pass, and neither brings 1,000 apps under the budget. A later keystroke that adds a character can align only where the previous one aligned, because every query character must align, so frizbee could run over the previous keystroke's aligned haystacks alone. Timed alone on 1,000 apps, that left `te` at 192 to 196 µs, `in` at 178 µs, `se` at 172 µs, `to` at 167 µs, and `ter` at 147 µs. With frizbee at no cost, the rest of `Index::rows` still took 94 to 98 µs for `te`, 93 µs for `se`, 87 µs for `in`, and 81 µs for `to`. A one-letter query shows only entries with that letter at a word start, so an index of word-start letters could narrow the first keystroke. `e` over its 729 word-start haystacks came to about 96 µs alone, and `e` cost about 50 µs more inside the bench's loop than alone (measured, 2026-10-16). A 1,000-app desktop would need both levers and a cheaper remainder. The 1,024-byte query cap does not bound a paste against an equally long field, because a field keeps as many bytes.

## The fixture formats are strict

The tests read every fenced block whose info string starts with a fixture kind. Words after the kind are arguments. Fences follow CommonMark, so a closing fence may be indented up to 3 spaces, be longer than its opening, or end in spaces and tabs, and a `~~~` or indented prose fence is read as prose. Any other character after a closing fence, such as a no-break space, makes the line part of the block, as CommonMark reads it. CommonMark reads no fence inside an HTML block, so the page puts no fixture in one, and a line outside a block that starts with `<` after up to 3 spaces fails the test. A fence whose first word is neither a fixture kind nor `rust`, `sh`, `text`, or nothing fails the test, so a misspelled kind cannot drop its block. So does a fence that names a kind and is not an unindented ```` ``` ````, such as one indented, longer, or of tildes. Any fence in a list item or blockquote fails the test too, because the reader does not follow those containers. So does a fence that names a kind inside another block, a block that never closes, and a carriage return, which CommonMark reads as a line ending and the reader does not. The CLI test checks that the `desktop` blocks are the files in the table above and that every `cli` command it expects exists, and the rank test checks the tags and the count of rank blocks. The other tests check that a few known lines exist.

| Kind | Lines | Test |
|---|---|---|
| `score` | `query \| text => N shows`, `N below`, `exact`, or `∅` | `lyncast-core/tests/ranking.rs` |
| `fold` | `input => frizbee's form \| lowercased` | `lyncast-core/tests/ranking.rs` |
| `rank rule=N`, `rank empty-list`, `rank keywords`, `rank separators`, `rank whitespace` | one scenario per block | `lyncast-core/tests/ranking.rs` |
| `frecency` | `visits <offsets> \| at <offset> => <score>` | `lyncast-core/tests/ranking.rs` |
| `terms` | `"query" ... => term \| term \| term` | `lyncast-core/tests/ranking.rs` |
| `entry locale=... desktop=...`, then `parsed` | a file's text, then its verdict | `lyncast-core/tests/desktop.rs` |
| `exec`, `exec no-icon` | `value => argv` or `value => error: ExecError` | `lyncast-core/tests/desktop.rs` |
| `env`, `desktop <path> [mode=<octal>]`, `executable <path>`, `symlink <path> -> <target>`, `fifo <path>`, `cli` | the CLI tree, its environment, and the expected stdout | `lyncast/tests/query.rs` |

A `rank` block has these lines:

- `app <id> | Key=Value | ...` adds an app. The test writes the cells under `[Desktop Entry]` after `Type=Application` and `Exec=true`, and runs the real parser.
- `session LANG=... XDG_CURRENT_DESKTOP=...` sets the parse session. Without it, the locale is `C` and the desktop is `niri`.
- `visit <key> <day> [query]` records a pick, in order. The key parses as `Key` does, and the query is the rest of the line.
- `now <day>` sets the clock, and `query <text>` the query. A day is 86,400,000 ms.
- `<section> => <titles>` lists the rows of one section in order, or `none`.

Every `rank` block has one tag. `rule=N` names the TinyCast rule the block pins. `empty-list`, `keywords`, `separators`, and `whitespace` tag the blocks for the empty list, a command's keywords, a query's separators, and text that is the empty query. The test fails when a block has no tag or an unknown one, when a kept rule (3, 4, 5, and 7 to 14) or a named tag has no block, or when the page does not hold 51 rank blocks. Some blocks are the only pin of part of a rule, so a lost block fails.

On both sides of a `fold` line, in an `entry` block, in the value of an `exec` line, and in a `rank` query line, the exact pattern `\u{XXXX}` writes an invisible character, such as `\u{A}` for a newline. Every other backslash stays, so `C:\Users` reads as written. In `desktop` blocks and in a `cli` command's variable values, the exact pattern `\x{HH}` writes the byte `HH`, so a file or a variable can hold a byte that is not UTF-8. A `cli` variable value may also hold `$TREE`, as the `env` block does. In a `rank` query or visit query, a `terms` line, a `cli` argument, and both sides of a `score` pair, `""` is the empty string, and a value in double quotes is read without them, so `"Firefox "` keeps its trailing space.
