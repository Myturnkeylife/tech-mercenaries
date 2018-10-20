extern crate toml;
extern crate serde_json;

use utils::fs::{read_file, is_file_in_directory};

use std::path::PathBuf;

use csv::Reader;
use std::collections::HashMap;
use tera::{GlobalFn, Value, from_value, to_value, Result, Map};
use std::ops::BitXor;

static GET_DATA_ARGUMENT_ERROR_MESSAGE: &str = "`load_data`: requires EITHER a `path` or `url` argument";

enum ProvidedArgument {
    URL(String),
    PATH(PathBuf)
}

fn get_data_from_args(args: &HashMap<String, Value>) -> Result<ProvidedArgument> {
    let path_arg = optional_arg!(
        String,
        args.get("path"),
        GET_DATA_ARGUMENT_ERROR_MESSAGE
    );

    let url_arg = optional_arg!(
        String,
        args.get("url"),
        GET_DATA_ARGUMENT_ERROR_MESSAGE
    );

    if !path_arg.is_some().bitxor(url_arg.is_some()) {
        return Err("GET_DATA_ARGUMENT_ERROR_MESSAGE.into()".into());
    }

    if let Some(path) = path_arg {
        return Ok(ProvidedArgument::PATH(PathBuf::from(path)));
    }
    else if let Some(url) = url_arg {
        return Ok(ProvidedArgument::URL(url));
    }

    return Err(GET_DATA_ARGUMENT_ERROR_MESSAGE.into());
}

fn read_data_file(content_path: &PathBuf, path_arg: PathBuf) -> Result<String> {
    let full_path = content_path.join(&path_arg);
    if !is_file_in_directory(&content_path, &path_arg).map_err(|e| format!("Failed to read data file {}: {}", full_path.display(), e))? {
        return Err(format!("{} is not inside the content directory {}", full_path.display(), content_path.display()).into());
    }
    return read_file(&full_path)
        .map_err(|e| format!("`load_data`: error {} loading file {}", full_path.to_str().unwrap(), e).into());
}

fn get_output_kind_from_args(args: &HashMap<String, Value>, provided_argument: &ProvidedArgument) -> Result<String> {
    let kind_arg = optional_arg!(
        String,
        args.get("kind"),
        "`load_data`: `kind` needs to be an argument with a string value, being one of the supported `load_data` file types (csv, json, toml)"
    );

    if let Some(kind) = kind_arg {
        return Ok(kind);
    }
    return match provided_argument {
        ProvidedArgument::PATH(path) => path.extension().map(|extension| extension.to_str().unwrap().to_string()).ok_or(format!("Could not determine kind for {} from extension", path.display()).into()),
        _ => Ok(String::from("plain"))
    }
}

/// A global function to load data from a data file.
/// Currently the supported formats are json, toml and csv
pub fn make_load_data(content_path: PathBuf) -> GlobalFn {
    Box::new(move |args| -> Result<Value> {


        let provided_argument = get_data_from_args(&args)?;

        let file_kind = get_output_kind_from_args(&args, &provided_argument)?;

        let data = match provided_argument {
            ProvidedArgument::PATH(path) => read_data_file(&content_path, path),
            ProvidedArgument::URL(_url) => Ok(String::from("test")),
        }?;

        let result_value: Result<Value> = match file_kind.as_str() {
            "toml" => load_toml(data),
            "csv" => load_csv(data),
            "json" => load_json(data),
            "plain" => to_value(data).map_err(|e| e.into()),
            kind => return Err(format!("'load_data': {} is an unsupported file kind", kind).into())
        };

        result_value
    })
}

/// load/parse a json file from the given path and place it into a
/// tera value
fn load_json(json_data: String) -> Result<Value> {
    let json_content = serde_json::from_str(json_data.as_str()).unwrap();
    let tera_value: Value = json_content;

    return Ok(tera_value);
}

/// load/parse a toml file from the given path, and place it into a
/// tera Value
fn load_toml(toml_data: String) -> Result<Value> {
    let toml_content: toml::Value = toml::from_str(&toml_data).map_err(|e| format!("{:?}", e))?;

    to_value(toml_content).map_err(|e| e.into())
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
fn load_csv(csv_data: String) -> Result<Value> {
    let mut reader = Reader::from_reader(csv_data.as_bytes());

    let mut csv_map = Map::new();

    {
        let hdrs = reader.headers()
            .map_err(|e| format!("'load_data': {} - unable to read CSV header line (line 1) for CSV file", e))?;

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
    use super::make_load_data;

    use std::collections::HashMap;
    use std::path::PathBuf;

    use tera::to_value;

    #[test]
    fn cant_load_outside_content_dir() {
        let static_fn = make_load_data(PathBuf::from("../utils/test-files"));
        let mut args = HashMap::new();
        args.insert("path".to_string(), to_value("../../../README.md").unwrap());
        let result = static_fn(args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().description(), "../utils/test-files/../../../README.md is not inside the content directory ../utils/test-files");
    }

    #[test]
    fn can_load_toml()
    {
        let static_fn = make_load_data(PathBuf::from("../utils/test-files"));
        let mut args = HashMap::new();
        args.insert("path".to_string(), to_value("test.toml").unwrap());
        let result = static_fn(args.clone()).unwrap();

        //TOML does not load in order, and also dates are not returned as strings, but
        //rather as another object with a key and value
        assert_eq!(result, json!({
            "category": {
                "date": {
                    "$__toml_private_datetime": "1979-05-27T07:32:00Z"
                },
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
