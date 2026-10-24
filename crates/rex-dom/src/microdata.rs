//! Microdata (HTML Living Standard §5.2) items as JSON objects, so the JSON-LD path machinery can
//! read them. `@type` is the short name of the first itemtype token.

use scraper::{ElementRef, Selector};
use serde_json::{Map, Value};

use crate::{text, Budget, Document};

/// Budget for building the microdata index (it is built once per document, outside recipe fuel).
const MICRODATA_FUEL: u64 = 2_000_000;

pub fn extract_items(doc: &Document) -> Vec<Value> {
    let budget = Budget::new(MICRODATA_FUEL);
    let sel = Selector::parse("[itemscope]").expect("static selector");
    let mut out = Vec::new();
    for el in doc.root().select(&sel) {
        // Top-level items only: an itemscope that is itself a property belongs to its parent item.
        if el.value().attr("itemprop").is_some() {
            continue;
        }
        if let Some(item) = build_item(doc, el, &budget, 0) {
            out.push(item);
        }
    }
    out
}

fn build_item(doc: &Document, item: ElementRef<'_>, budget: &Budget, depth: u32) -> Option<Value> {
    if depth > 16 || budget.spend(1).is_err() {
        return None;
    }
    let mut obj = Map::new();
    if let Some(t) = item
        .value()
        .attr("itemtype")
        .and_then(|t| t.split_ascii_whitespace().next())
    {
        let short = t.rsplit(['/', '#']).next().unwrap_or(t);
        obj.insert("@type".into(), Value::String(short.to_string()));
    }
    collect_props(doc, item, &mut obj, budget, depth);
    Some(Value::Object(obj))
}

fn collect_props(doc: &Document, el: ElementRef<'_>, obj: &mut Map<String, Value>, budget: &Budget, depth: u32) {
    for child in el.children().filter_map(ElementRef::wrap) {
        if budget.spend(1).is_err() {
            return;
        }
        let props = child
            .value()
            .attr("itemprop")
            .map(|p| p.split_ascii_whitespace().map(str::to_string).collect::<Vec<_>>());
        let nested_scope = child.value().attr("itemscope").is_some();
        if let Some(names) = props {
            let value = if nested_scope {
                build_item(doc, child, budget, depth + 1).unwrap_or(Value::Null)
            } else {
                Value::String(prop_value(doc, child, budget))
            };
            for name in names {
                match obj.get_mut(&name) {
                    Some(Value::Array(a)) => a.push(value.clone()),
                    Some(existing) => {
                        let prev = existing.take();
                        *existing = Value::Array(vec![prev, value.clone()]);
                    }
                    None => {
                        obj.insert(name, value.clone());
                    }
                }
            }
        }
        if !nested_scope {
            collect_props(doc, child, obj, budget, depth);
        }
    }
}

fn prop_value(doc: &Document, el: ElementRef<'_>, budget: &Budget) -> String {
    let e = el.value();
    let url_attr = |a: &str| {
        e.attr(a)
            .map(|v| {
                doc.resolve_url(v)
                    .map(|u| u.to_string())
                    .unwrap_or_else(|| v.to_string())
            })
            .unwrap_or_default()
    };
    match e.name() {
        "meta" => e.attr("content").unwrap_or_default().to_string(),
        "audio" | "embed" | "iframe" | "img" | "source" | "track" | "video" => url_attr("src"),
        "a" | "area" | "link" => url_attr("href"),
        "object" => url_attr("data"),
        "data" | "meter" => e.attr("value").unwrap_or_default().to_string(),
        "time" => e
            .attr("datetime")
            .map(str::to_string)
            .unwrap_or_else(|| text::string_text(el, budget).unwrap_or_default()),
        _ => e
            .attr("content")
            .map(str::to_string)
            .unwrap_or_else(|| text::string_text(el, budget).unwrap_or_default()),
    }
}

#[cfg(test)]
mod tests {
    use crate::{Document, Limits};

    #[test]
    fn nested_items_and_typed_values() {
        // Arrange
        let d = Document::parse(
            r#"<div itemscope itemtype="https://schema.org/Product">
                 <h1 itemprop="name">Widget</h1>
                 <img itemprop="image" src="/w.png">
                 <div itemprop="offers" itemscope itemtype="https://schema.org/Offer">
                   <meta itemprop="price" content="19.99"><span itemprop="priceCurrency">USD</span>
                 </div></div>"#,
            "https://shop.example/p/1",
            &Limits::default(),
        )
        .unwrap();

        // Act
        let items = d.microdata();

        // Assert
        assert_eq!(items.len(), 1);
        let p = &items[0];
        assert_eq!(p["@type"], "Product");
        assert_eq!(p["name"], "Widget");
        assert_eq!(p["image"], "https://shop.example/w.png");
        assert_eq!(p["offers"]["@type"], "Offer");
        assert_eq!(p["offers"]["price"], "19.99");
    }
}
