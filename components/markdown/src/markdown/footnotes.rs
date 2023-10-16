use std::collections::HashMap;

use libs::pulldown_cmark::{escape::StrWrite, CowStr, Event, Tag};

/// Extracts the footnotes as bottom-notes, in the style of GitHub.
///
/// Adapted from [notriddle](https://github.com/raphlinus/pulldown-cmark/blob/ad19879e6b4f411dbdd78ffa6a9cf429e35afe47/examples/footnote-rewrite.rs).
pub(crate) fn render_bottom_footnotes(events: Vec<Event<'_>>) -> Vec<Event<'_>> {
    // To generate this style, you have to collect the footnotes at the end, while parsing.
    // You also need to count usages.
    let mut footnotes = Vec::new();
    let mut in_footnote = Vec::new();
    let mut footnote_numbers = HashMap::new();
    let mut events_with_footnotes: Vec<_> = events
        .into_iter()
        .filter_map(|event|{
            match event {
                Event::Start(Tag::FootnoteDefinition(_)) => {
                    in_footnote.push(vec![event]);
                    None
                }
                Event::End(Tag::FootnoteDefinition(_)) => {
                    let mut f = in_footnote.pop().unwrap();
                    f.push(event);
                    footnotes.push(f);
                    None
                }
                Event::FootnoteReference(name) => {
                    let n = footnote_numbers.len() + 1;
                    let (n, nr) = footnote_numbers.entry(name.clone()).or_insert((n, 0usize));
                    *nr += 1;
                    let html = Event::Html(format!(r##"<sup class="footnote-reference" id="fr-{name}-{nr}"><a href="#fn-{name}">[{n}]</a></sup>"##).into());
                    if in_footnote.is_empty() {
                        Some(html)
                    } else {
                        in_footnote.last_mut().unwrap().push(html);
                        None
                    }
                }
                _ if !in_footnote.is_empty() => {
                    in_footnote.last_mut().unwrap().push(event);
                    None
                }
                _ => Some(event)
            }
        }).collect();

    // To make the footnotes look right, we need to sort them by their appearance order, not by
    // the in-tree order of their actual definitions. Unused items are omitted entirely.
    //
    // For example, this code:
    //
    //     test [^1] [^2]
    //     [^2]: second used, first defined
    //     [^1]: test
    //
    // Gets rendered like *this* if you copy it into a GitHub comment box:
    //
    //     <p>test <sup>[1]</sup> <sup>[2]</sup></p>
    //     <hr>
    //     <ol>
    //     <li>test ↩</li>
    //     <li>second used, first defined ↩</li>
    //     </ol>

    if !footnotes.is_empty() {
        footnotes.retain(|f| match f.first() {
            Some(Event::Start(Tag::FootnoteDefinition(name))) => {
                footnote_numbers.get(name).unwrap_or(&(0, 0)).1 != 0
            }
            _ => false,
        });
        footnotes.sort_by_cached_key(|f| match f.first() {
            Some(Event::Start(Tag::FootnoteDefinition(name))) => {
                footnote_numbers.get(name).unwrap_or(&(0, 0)).0
            }
            _ => unreachable!(),
        });
        events_with_footnotes.push(Event::Html("<hr><ol class=\"footnotes-list\">".into()));

        events_with_footnotes.extend(footnotes.into_iter().flat_map(|fl| {
            // To write backrefs, the name needs kept until the end of the footnote definition.
            let mut name = CowStr::from("");
            // Backrefs are included in the final paragraph of the footnote, if it's normal text.
            // For example, this DOM can be produced:
            //
            // Markdown:
            //
            //     five [^feet].
            //
            //     [^feet]:
            //         A foot is defined, in this case, as 0.3048 m.
            //
            //         Historically, the foot has not been defined this way, corresponding to many
            //         subtly different units depending on the location.
            //
            // HTML:
            //
            //     <p>five <sup class="footnote-reference" id="fr-feet-1"><a href="#fn-feet">[1]</a></sup>.</p>
            //
            //     <ol class="footnotes-list">
            //     <li id="fn-feet">
            //     <p>A foot is defined, in this case, as 0.3048 m.</p>
            //     <p>Historically, the foot has not been defined this way, corresponding to many
            //     subtly different units depending on the location. <a href="#fr-feet-1">↩</a></p>
            //     </li>
            //     </ol>
            //
            // This is mostly a visual hack, so that footnotes use less vertical space.
            //
            // If there is no final paragraph, such as a tabular, list, or image footnote, it gets
            // pushed after the last tag instead.
            let mut has_written_backrefs = false;
            let fl_len = fl.len();
            let footnote_numbers = &footnote_numbers;
            fl.into_iter().enumerate().map(move |(i, f)| match f {
                Event::Start(Tag::FootnoteDefinition(current_name)) => {
                    name = current_name;
                    has_written_backrefs = false;
                    Event::Html(format!(r##"<li id="fn-{name}">"##).into())
                }
                Event::End(Tag::FootnoteDefinition(_) | Tag::Paragraph)
                    if !has_written_backrefs && i >= fl_len - 2 =>
                {
                    let usage_count = footnote_numbers.get(&name).unwrap().1;
                    let mut end = String::with_capacity(
                        name.len() + (r##" <a href="#fr--1">↩</a></li>"##.len() * usage_count),
                    );
                    for usage in 1..=usage_count {
                        if usage == 1 {
                            write!(&mut end, r##" <a href="#fr-{name}-{usage}">↩</a>"##).unwrap();
                        } else {
                            write!(&mut end, r##" <a href="#fr-{name}-{usage}">↩{usage}</a>"##)
                                .unwrap();
                        }
                    }
                    has_written_backrefs = true;
                    if matches!(f, Event::End(Tag::FootnoteDefinition(_))) {
                        end.push_str("</li>\n");
                    } else {
                        end.push_str("</p>\n");
                    }
                    Event::Html(end.into())
                }
                Event::End(Tag::FootnoteDefinition(_)) => Event::Html("</li>\n".into()),
                Event::FootnoteReference(_) => unreachable!("converted to HTML earlier"),
                f => f,
            })
        }));

        events_with_footnotes.push(Event::Html("</ol>".into()));
    }

    // No footnotes
    events_with_footnotes
}
