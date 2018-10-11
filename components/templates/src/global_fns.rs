extern crate toml;
extern crate serde_json;
extern crate error_chain;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::path::PathBuf;

use csv::Reader;

use tera::{GlobalFn, Value, from_value, to_value, Result, Map, Error, ErrorKind};
use self::error_chain::ChainedError;

use library::{Taxonomy, Library};
use config::Config;
use utils::site::resolve_internal_link;
use utils::fs::read_file;
use imageproc;
use errors;


macro_rules! required_arg {
    ($ty: ty, $e: expr, $err: expr) => {
        match $e {
            Some(v) => match from_value::<$ty>(v.clone()) {
                Ok(u) => u,
                Err(_) => return Err($err.into())
            },
            None => return Err($err.into())
        }
    };
}

macro_rules! optional_arg {
    ($ty: ty, $e: expr, $err: expr) => {
        match $e {
            Some(v) => match from_value::<$ty>(v.clone()) {
                Ok(u) => Some(u),
                Err(_) => return Err($err.into())
            },
            None => None
        }
    };
}


pub fn make_trans(config: Config) -> GlobalFn {
    let translations_config = config.translations;
    let default_lang = config.default_language.clone();

    Box::new(move |args| -> Result<Value> {
        let key = required_arg!(String, args.get("key"), "`trans` requires a `key` argument.");
        let lang = optional_arg!(
            String,
            args.get("lang"),
            "`trans`: `lang` must be a string."
        ).unwrap_or_else(|| default_lang.clone());
        let translations = &translations_config[lang.as_str()];
        Ok(to_value(&translations[key.as_str()]).unwrap())
    })
}


pub fn make_get_page(library: &Library) -> GlobalFn {
    let mut pages = HashMap::new();
    for page in library.pages_values() {
        pages.insert(
            page.file.relative.clone(),
            to_value(library.get_page(&page.file.path).unwrap().to_serialized(library.pages())).unwrap(),
        );
    }

    Box::new(move |args| -> Result<Value> {
        let path = required_arg!(
            String,
            args.get("path"),
            "`get_page` requires a `path` argument with a string value"
        );
        match pages.get(&path) {
            Some(p) => Ok(p.clone()),
            None => Err(format!("Page `{}` not found.", path).into())
        }
    })
}

pub fn make_get_section(library: &Library) -> GlobalFn {
    let mut sections = HashMap::new();
    for section in library.sections_values() {
        sections.insert(
            section.file.relative.clone(),
            to_value(library.get_section(&section.file.path).unwrap().to_serialized(library)).unwrap(),
        );
    }

    Box::new(move |args| -> Result<Value> {
        let path = required_arg!(
            String,
            args.get("path"),
            "`get_section` requires a `path` argument with a string value"
        );

        match sections.get(&path) {
            Some(p) => Ok(p.clone()),
            None => Err(format!("Section `{}` not found.", path).into())
        }
    })
}

pub fn make_get_url(permalinks: HashMap<String, String>, config: Config) -> GlobalFn {
    Box::new(move |args| -> Result<Value> {
        let cachebust = args
            .get("cachebust")
            .map_or(false, |c| {
                from_value::<bool>(c.clone()).unwrap_or(false)
            });

        let trailing_slash = args
            .get("trailing_slash")
            .map_or(false, |c| {
                from_value::<bool>(c.clone()).unwrap_or(false)
            });

        let path = required_arg!(
            String,
            args.get("path"),
            "`get_url` requires a `path` argument with a string value"
        );
        if path.starts_with("./") {
            match resolve_internal_link(&path, &permalinks) {
                Ok(url) => Ok(to_value(url).unwrap()),
                Err(_) => Err(format!("Could not resolve URL for link `{}` not found.", path).into())
            }
        } else {
            // anything else
            let mut permalink = config.make_permalink(&path);
            if !trailing_slash && permalink.ends_with('/') {
                permalink.pop(); // Removes the slash
            }

            if cachebust {
                permalink = format!("{}?t={}", permalink, config.build_timestamp.unwrap());
            }
            Ok(to_value(permalink).unwrap())
        }
    })
}

pub fn make_get_taxonomy(all_taxonomies: &[Taxonomy], library: &Library) -> GlobalFn {
    let mut taxonomies = HashMap::new();
    for taxonomy in all_taxonomies {
        taxonomies.insert(
            taxonomy.kind.name.clone(),
            to_value(taxonomy.to_serialized(library)).unwrap()
        );
    }

    Box::new(move |args| -> Result<Value> {
        let kind = required_arg!(
            String,
            args.get("kind"),
            "`get_taxonomy` requires a `kind` argument with a string value"
        );
        let container = match taxonomies.get(&kind) {
            Some(c) => c,
            None => return Err(
                format!("`get_taxonomy` received an unknown taxonomy as kind: {}", kind).into()
            ),
        };

        Ok(to_value(container).unwrap())
    })
}

pub fn make_get_taxonomy_url(all_taxonomies: &[Taxonomy]) -> GlobalFn {
    let mut taxonomies = HashMap::new();
    for taxonomy in all_taxonomies {
        let mut items = HashMap::new();
        for item in &taxonomy.items {
            items.insert(item.name.clone(), item.permalink.clone());
        }
        taxonomies.insert(taxonomy.kind.name.clone(), items);
    }

    Box::new(move |args| -> Result<Value> {
        let kind = required_arg!(
            String,
            args.get("kind"),
            "`get_taxonomy_url` requires a `kind` argument with a string value"
        );
        let name = required_arg!(
            String,
            args.get("name"),
            "`get_taxonomy_url` requires a `name` argument with a string value"
        );
        let container = match taxonomies.get(&kind) {
            Some(c) => c,
            None => return Err(
                format!("`get_taxonomy_url` received an unknown taxonomy as kind: {}", kind).into()
            )
        };

        if let Some(ref permalink) = container.get(&name) {
            return Ok(to_value(permalink.clone()).unwrap());
        }

        Err(
            format!("`get_taxonomy_url`: couldn't find `{}` in `{}` taxonomy", name, kind).into()
        )
    })
}

pub fn make_resize_image(imageproc: Arc<Mutex<imageproc::Processor>>) -> GlobalFn {
    static DEFAULT_OP: &'static str = "fill";
    const DEFAULT_Q: u8 = 75;

    Box::new(move |args| -> Result<Value> {
        let path = required_arg!(
            String,
            args.get("path"),
            "`resize_image` requires a `path` argument with a string value"
        );
        let width = optional_arg!(
            u32,
            args.get("width"),
            "`resize_image`: `width` must be a non-negative integer"
        );
        let height = optional_arg!(
            u32,
            args.get("height"),
            "`resize_image`: `height` must be a non-negative integer"
        );
        let op = optional_arg!(
            String,
            args.get("op"),
            "`resize_image`: `op` must be a string"
        ).unwrap_or_else(|| DEFAULT_OP.to_string());
        let quality = optional_arg!(
            u8,
            args.get("quality"),
            "`resize_image`: `quality` must be a number"
        ).unwrap_or(DEFAULT_Q);
        if quality == 0 || quality > 100 {
            return Err("`resize_image`: `quality` must be in range 1-100".to_string().into());
        }

        let mut imageproc = imageproc.lock().unwrap();
        if !imageproc.source_exists(&path) {
            return Err(format!("`resize_image`: Cannot find path: {}", path).into());
        }

        let imageop = imageproc::ImageOp::from_args(path.clone(), &op, width, height, quality)
            .map_err(|e| format!("`resize_image`: {}", e))?;
        let url = imageproc.insert(imageop);

        to_value(url).map_err(|err| err.into())
    })
}

/// A global function to load data from a data file.
/// Currently the supported formats are json, toml and csv
pub fn make_load_data(content_path: PathBuf) -> GlobalFn {
    Box::new(move |args| -> Result<Value> {
        let path_arg: String = required_arg!(
            String, 
            args.get("path"), 
            "`load_data`: requires a `path` argument with a string value, being a path to a file"
        );
        let kind_arg = optional_arg!(
            String,
            args.get("kind"),
            "`load_data`: `kind` needs to be an argument with a string value, being one of the supported `load_data` file types (csv, json, toml)"
        );

        let full_path = content_path.join(&path_arg);

        let extension = match full_path.extension() {
            Some(value) => value.to_str().unwrap().to_lowercase(),
            None => return Err(format!("`load_data`: Cannot parse file extension of specified file: {}", path_arg).into())
        };

        let file_kind = kind_arg.unwrap_or(extension);

        let result_value: Result<Value> = match file_kind.as_str() {
            "toml" => load_toml(&full_path),
            "csv" => load_csv(&full_path),
            "json" => load_json(&full_path),
            _ => return Err(format!("'load_data': {} - is an unsupported file kind", file_kind).into())
        };

        result_value
    })
}

/// load/parse a json file from the given path and place it into a 
/// tera value
fn load_json(json_path: &PathBuf) -> Result<Value> {
    
    let content_string: String = read_file(json_path)
        .chain_err(|| format!("`load_data`: error loading json file {}", json_path.to_str().unwrap().into()))?;

    let json_content = serde_json::from_str(content_string.as_str()).unwrap();
    let tera_value: Value = json_content;

    return Ok(tera_value);
}

/// load/parse a toml file from the given path, and place it into a
/// tera Value
fn load_toml(toml_path: &PathBuf) -> Result<Value> {
    let content_string: String = read_file(toml_path)
        .map_err(|err: errors::Error| {
                Error::with_chain(err, ErrorKind::Msg(String::from("`load_data`: error loading toml file")))
            })?;

    let toml_content: toml::Value = toml::from_str(&content_string)
        .map_err(|e| format!("'load_data': {} - {}", toml_path.to_str().unwrap(), e))?;

    to_value(toml_content).map_err(|err| err.into())
}

/// Load/parse a csv file from the given path, and place it into a 
/// tera Value.
/// 
/// An example csv file `example.csv` could be:
/// ```csv
/// Number, Title
/// 1,Gutenberg
/// 2,Printing
/// ```
/// The json value output would be:
/// ```json
/// {
///     "headers": ["Number", "Title"],
///     "records": [
///                     ["1", "Gutenberg"], 
///                     ["2", "Printing"]
///                ],
/// }
/// ```
fn load_csv(csv_path: &PathBuf) -> Result<Value> {
    let mut reader = Reader::from_path(csv_path.clone())
        .map_err(|e| format!("'load_data': {} - {}", csv_path.to_str().unwrap(), e))?;

    let mut csv_map = Map::new();

    {
        let hdrs = reader.headers()
            .map_err(|e| format!("'load_data': {} - {} - unable to read CSV header line (line 1) for CSV file", csv_path.to_str().unwrap(), e))?;

        let headers_array = hdrs.iter()
            .map(|v| Value::String(v.to_string()))
            .collect();

        csv_map.insert(String::from("headers"), Value::Array(headers_array));
    }

    {
        let records = reader.records();

        let mut records_array: Vec<Value> = Vec::new();

        for result in records {
            let record = result.unwrap();

            let mut elements_array: Vec<Value> = Vec::new();

            for e in record.into_iter() {
                elements_array.push(Value::String(String::from(e)));
            }

            records_array.push(Value::Array(elements_array));
        }

        csv_map.insert(String::from("records"), Value::Array(records_array));
    }

    let csv_value: Value = Value::Object(csv_map);
    to_value(csv_value).map_err(|err| err.into())
}

#[cfg(test)]
mod tests {
    use super::{make_get_url, make_get_taxonomy, make_get_taxonomy_url, make_trans, make_load_data};

    use std::collections::HashMap;
    use std::path::PathBuf;

    use tera::{to_value, Value};

    use config::{Config, Taxonomy as TaxonomyConfig};
    use library::{Taxonomy, TaxonomyItem, Library};


    #[test]
    fn can_add_cachebust_to_url() {
        let config = Config::default();
        let static_fn = make_get_url(HashMap::new(), config);
        let mut args = HashMap::new();
        args.insert("path".to_string(), to_value("app.css").unwrap());
        args.insert("cachebust".to_string(), to_value(true).unwrap());
        assert_eq!(static_fn(args).unwrap(), "http://a-website.com/app.css?t=1");
    }

    #[test]
    fn can_add_trailing_slashes() {
        let config = Config::default();
        let static_fn = make_get_url(HashMap::new(), config);
        let mut args = HashMap::new();
        args.insert("path".to_string(), to_value("app.css").unwrap());
        args.insert("trailing_slash".to_string(), to_value(true).unwrap());
        assert_eq!(static_fn(args).unwrap(), "http://a-website.com/app.css/");
    }

    #[test]
    fn can_add_slashes_and_cachebust() {
        let config = Config::default();
        let static_fn = make_get_url(HashMap::new(), config);
        let mut args = HashMap::new();
        args.insert("path".to_string(), to_value("app.css").unwrap());
        args.insert("trailing_slash".to_string(), to_value(true).unwrap());
        args.insert("cachebust".to_string(), to_value(true).unwrap());
        assert_eq!(static_fn(args).unwrap(), "http://a-website.com/app.css/?t=1");
    }

    #[test]
    fn can_link_to_some_static_file() {
        let config = Config::default();
        let static_fn = make_get_url(HashMap::new(), config);
        let mut args = HashMap::new();
        args.insert("path".to_string(), to_value("app.css").unwrap());
        assert_eq!(static_fn(args).unwrap(), "http://a-website.com/app.css");
    }

    #[test]
    fn can_get_taxonomy() {
        let taxo_config = TaxonomyConfig { name: "tags".to_string(), ..TaxonomyConfig::default() };
        let library = Library::new(0, 0);
        let tag = TaxonomyItem::new(
            "Programming",
            "tags",
            &Config::default(),
            vec![],
            &library
        );
        let tags = Taxonomy {
            kind: taxo_config,
            items: vec![tag],
        };

        let taxonomies = vec![tags.clone()];
        let static_fn = make_get_taxonomy(&taxonomies, &library);
        // can find it correctly
        let mut args = HashMap::new();
        args.insert("kind".to_string(), to_value("tags").unwrap());
        let res = static_fn(args).unwrap();
        let res_obj = res.as_object().unwrap();
        assert_eq!(res_obj["kind"], to_value(tags.kind).unwrap());
        assert_eq!(res_obj["items"].clone().as_array().unwrap().len(), 1);
        assert_eq!(
            res_obj["items"].clone().as_array().unwrap()[0].clone().as_object().unwrap()["name"],
            Value::String("Programming".to_string())
        );
        assert_eq!(
            res_obj["items"].clone().as_array().unwrap()[0].clone().as_object().unwrap()["slug"],
            Value::String("programming".to_string())
        );
        assert_eq!(
            res_obj["items"].clone().as_array().unwrap()[0].clone().as_object().unwrap()["permalink"],
            Value::String("http://a-website.com/tags/programming/".to_string())
        );
        assert_eq!(
            res_obj["items"].clone().as_array().unwrap()[0].clone().as_object().unwrap()["pages"],
            Value::Array(vec![])
        );
        // and errors if it can't find it
        let mut args = HashMap::new();
        args.insert("kind".to_string(), to_value("something-else").unwrap());
        assert!(static_fn(args).is_err());
    }

    #[test]
    fn can_get_taxonomy_url() {
        let taxo_config = TaxonomyConfig { name: "tags".to_string(), ..TaxonomyConfig::default() };
        let library = Library::new(0, 0);
        let tag = TaxonomyItem::new(
            "Programming",
            "tags",
            &Config::default(),
            vec![],
            &library
        );
        let tags = Taxonomy {
            kind: taxo_config,
            items: vec![tag],
        };

        let taxonomies = vec![tags.clone()];
        let static_fn = make_get_taxonomy_url(&taxonomies);
        // can find it correctly
        let mut args = HashMap::new();
        args.insert("kind".to_string(), to_value("tags").unwrap());
        args.insert("name".to_string(), to_value("Programming").unwrap());
        assert_eq!(static_fn(args).unwrap(), to_value("http://a-website.com/tags/programming/").unwrap());
        // and errors if it can't find it
        let mut args = HashMap::new();
        args.insert("kind".to_string(), to_value("tags").unwrap());
        args.insert("name".to_string(), to_value("random").unwrap());
        assert!(static_fn(args).is_err());
    }

    #[test]
    fn can_translate_a_string() {
        let trans_config = r#"
base_url = "https://remplace-par-ton-url.fr"
default_language = "fr"

[translations]
[translations.fr]
title = "Un titre"

[translations.en]
title = "A title"

        "#;

        let config = Config::parse(trans_config).unwrap();
        let static_fn = make_trans(config);
        let mut args = HashMap::new();

        args.insert("key".to_string(), to_value("title").unwrap());
        assert_eq!(static_fn(args.clone()).unwrap(), "Un titre");

        args.insert("lang".to_string(), to_value("en").unwrap());
        assert_eq!(static_fn(args.clone()).unwrap(), "A title");

        args.insert("lang".to_string(), to_value("fr").unwrap());
        assert_eq!(static_fn(args.clone()).unwrap(), "Un titre");
    }

    #[test]
    fn can_load_toml()
    {
        let static_fn = make_load_data(PathBuf::from("../utils/test-files"));
        let mut args = HashMap::new();
        args.insert("path".to_string(), to_value("test.toml").unwrap());
        let result = static_fn(args.clone()).unwrap();

        assert_eq!(result, json!({
            "category": {
                "key": "value"
            },
        }));
    }

    #[test]
    fn can_load_csv()
    {
        let static_fn = make_load_data(PathBuf::from("../utils/test-files"));
        let mut args = HashMap::new();
        args.insert("path".to_string(), to_value("test.csv").unwrap());
        let result = static_fn(args.clone()).unwrap();

        assert_eq!(result, json!({
            "headers": ["Number", "Title"],
            "records": [
                            ["1", "Gutenberg"], 
                            ["2", "Printing"]
                        ],
        }))
    }

    #[test]
    fn can_load_json()
    {
        let static_fn = make_load_data(PathBuf::from("../utils/test-files"));
        let mut args = HashMap::new();
        args.insert("path".to_string(), to_value("test.json").unwrap());
        let result = static_fn(args.clone()).unwrap();

        assert_eq!(result, json!({
            "key": "value",
            "array": [1, 2, 3],
            "subpackage": {
                "subkey": 5
            }
        }))
    }
}
