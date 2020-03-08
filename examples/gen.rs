use heck::*;
use scraper::{ElementRef, Html, Selector};
use std::{collections::HashMap, fs::read_to_string, rc::Rc, str::FromStr};
use structopt::StructOpt;

#[derive(StructOpt)]
/// Generate Rust code from Keycloak REST Description in HTML
enum Cli {
    /// Generate types
    Types,
    /// Generate method callers
    Rest,
}

fn main() -> Result<(), std::io::Error> {
    let cli = Cli::from_args();
    match cli {
        Cli::Types => generate_types()?,
        Cli::Rest => generate_rest()?,
    }
    Ok(())
}

fn generate_rest() -> Result<(), std::io::Error> {
    let document = Html::parse_document(&read_to_string("./docs/rest-api-9.html")?);
    let (enums, structs, type_registry) = read_types_info(&document)?;
    let methods = read_methods_info(&document, &type_registry)?;
    write_rest(&enums, &structs, &type_registry);
    Ok(())
}

fn generate_types() -> Result<(), std::io::Error> {
    let document = Html::parse_document(&read_to_string("./docs/rest-api-9.html")?);
    let (enums, structs, type_registry) = read_types_info(&document)?;
    write_types(&enums, &structs, &type_registry);
    Ok(())
}

fn check_array(value: &str) -> Option<&str> {
    if value.starts_with("< ") && value.ends_with(" > array") {
        Some(&value[2..value.len() - 8])
    } else {
        None
    }
}

fn check_optional(value: &str) -> bool {
    "optional" == value
}

type TypeTrio = (
    Vec<EnumType>,
    Vec<Rc<StructType>>,
    HashMap<String, Rc<StructType>>,
);
fn read_types_info(document: &scraper::Html) -> Result<TypeTrio, std::io::Error> {
    let definitions_selector =
        Selector::parse("#_definitions ~ div.sectionbody > div.sect2").unwrap();

    let definitions = document.select(&definitions_selector);

    let mut rename_table = HashMap::new();
    rename_table.insert("Userinfo", "UserInfo".to_string());

    let mut structs = vec![];
    let mut type_registry = HashMap::new();
    let mut enums = vec![];
    for definition in definitions {
        let struct_name = text(&definition, "h3").replace("-", "");

        let fields_selector = Selector::parse("tbody tr").unwrap();

        let fields = definition.select(&fields_selector);
        let mut result_fields = vec![];
        let mut is_camel_case = false;
        for field in fields {
            let original_field = text(&field, "strong");
            let mut field_name = original_field.to_snake_case();
            let mut is_rename = false;
            if original_field != field_name {
                if field_name.to_mixed_case() == original_field {
                    is_camel_case = true;
                } else {
                    is_rename = true;
                }
            }

            let original_field_type = text(&field, "td ~ td p").replace("-", "");

            let array_field = check_array(&original_field_type);
            let is_array = array_field.is_some();

            let field_type = match if is_array {
                convert_type(array_field.unwrap())
            } else {
                convert_type(&original_field_type)
            } {
                Ok(field_type) => field_type,
                Err(ConvertTypeFail::Enum(enum_fields)) => {
                    let enum_name = format!("{}{}", struct_name, field_name.to_camel_case());

                    let is_upper_case = enum_fields
                        .split(", ")
                        .all(|x| x.chars().all(|x| x.is_uppercase()));
                    let enum_ = EnumType {
                        name: enum_name.clone(),
                        is_upper_case,
                        fields: enum_fields
                            .split(", ")
                            .map(|enum_field| {
                                let enum_field = enum_field.to_camel_case();
                                rename_table
                                    .get(enum_field.as_str())
                                    .unwrap_or_else(|| &enum_field)
                                    .clone()
                            })
                            .collect(),
                    };
                    enums.push(enum_);
                    FieldType::Simple(enum_name.into())
                }
                Err(err) => panic!("err: {:?}", err),
            };

            let optional_required = text(&field, "em");

            let is_optional = check_optional(&optional_required);

            if field_name == "type" || field_name == "self" {
                is_rename = true;
                field_name = format!("{}_", field_name);
            }

            let field = Field {
                field_name,
                original_field,
                field_type,
                is_array,
                is_optional,
                is_rename,
            };

            result_fields.push(field);
        }

        let struct_ = Rc::new(StructType {
            name: struct_name.clone(),
            is_camel_case,
            fields: result_fields,
        });
        type_registry.insert(struct_name, struct_.clone());
        structs.push(struct_);
    }

    Ok((enums, structs, type_registry))
}

fn read_methods_info(
    document: &scraper::Html,
    type_registry: &HashMap<String, Rc<StructType>>,
) -> Result<Vec<MethodStruct>, std::io::Error> {
    let resources_selector = Selector::parse("#_paths ~ div.sectionbody > div.sect2").unwrap();

    let resources_html = document.select(&resources_selector);
    let mut methods = vec![];
    for resource in resources_html {
        let resource_name = text(&resource, "h3");
        eprintln!("{}", resource_name);

        let methods_selector = Selector::parse("div.sect3").unwrap();

        let methods_html = resource.select(&methods_selector);
        for method in methods_html {
            let method_name = text(&method, "h4");
            let path = text_opt(&method, "pre").unwrap_or_else(|| method_name.clone());
            let mut path_parts = path.split(" ");
            let method_http = path_parts.next().unwrap();
            let path = path_parts.next().unwrap();
            eprintln!("{}", method_name);
            eprintln!("{:?}", path);

            let blocks_selector = Selector::parse("div.sect4").unwrap();
            let blocks_html = method.select(&blocks_selector);

            let mut parameters = vec![];
            let mut response = None;
            for block in blocks_html {
                let block_name = text(&block, "h5");
                eprintln!("{}", block_name);

                match block_name.as_str() {
                    "Parameters" => {
                        let parameters_selector = Selector::parse("tbody > tr").unwrap();
                        let parameters_html = block.select(&parameters_selector);

                        for parameter in parameters_html {
                            let parameter_kind = text(&parameter, "td:nth-child(1) > p > strong");
                            eprintln!("{}", parameter_kind);
                            let name = text(&parameter, "td:nth-child(2) > p > strong");
                            eprintln!("{}", name);
                            let optional_required = text(&parameter, "td:nth-child(2) > p > em");
                            eprintln!("{}", optional_required);
                            let comment = text_opt(&parameter, "td:nth-child(3) > p");
                            let parameter_type = text_opt(&parameter, "td:nth-child(4) > p")
                                .unwrap_or_else(|| {
                                    text_opt(&parameter, "td:last-child > p").unwrap()
                                });
                            eprintln!("{}", parameter_type);

                            let array = check_array(&parameter_type);

                            let is_optional = check_optional(&optional_required);
                            let parameter_ = Parameter {
                                name,
                                comment,
                                is_optional,
                                is_array: array.is_some(),
                                kind: parameter_kind.parse().unwrap(),
                                parameter_type: array
                                    .or_else(|| Some(parameter_type.as_str()))
                                    .map(convert_type)
                                    .unwrap()
                                    .unwrap(),
                            };
                            parameters.push(parameter_);
                        }
                    }
                    "Responses" => {
                        let response_type = text(&block, "tbody > tr > td:nth-child(3) > p");
                        eprintln!("{}", response_type);
                        let array = check_array(&response_type);
                        response = Some(ResponseType {
                            is_array: array.is_some(),
                            return_type: array
                                .or_else(|| Some(response_type.as_str()))
                                .map(convert_type)
                                .unwrap()
                                .unwrap(),
                        });
                    }
                    "Produces" => {}
                    _ => eprintln!("Unsupported block {}", block_name),
                }
            }

            let method = MethodStruct {
                comment: resource_name.clone(),
                name: method_name,
                parameters,
                path: path.into(),
                method: method_http.into(),
                response: response.unwrap(),
            };
            methods.push(method);
        }
    }

    Ok(methods)
}

fn write_types(
    enums: &[EnumType],
    structs: &[Rc<StructType>],
    type_registry: &HashMap<String, Rc<StructType>>,
) {
    println!("use serde::{{Deserialize, Serialize}};");
    println!("use serde_json::Value;");
    println!("use std::{{borrow::Cow, collections::HashMap}};\n");

    for e in enums {
        println!("#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]");
        if e.is_upper_case {
            println!(r#"#[serde(rename_all = "UPPERCASE")]"#);
        }

        println!("pub enum {} {{", e.name);

        for field in &e.fields {
            println!("    {},", field);
        }

        println!("}}\n");
    }

    for s in structs {
        let is_lifetime = s.is_lifetime(&type_registry);
        println!("#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]");
        if s.is_camel_case {
            println!(r#"#[serde(rename_all = "camelCase")]"#);
        }
        println!(
            "pub struct {}{} {{",
            s.name,
            if is_lifetime { "<'a>" } else { "" }
        );

        for field in &s.fields {
            if field.is_rename {
                println!(r#"    #[serde(rename = "{}")]"#, field.original_field);
            }
            let mut field_type = field.field_type.name(&type_registry);
            if field.is_array {
                field_type = format!("Vec<{}>", field_type);
            }
            if field.is_optional {
                field_type = format!("Option<{}>", field_type);
            }
            println!("    pub {}: {},", field.field_name, field_type);
        }

        println!("}}\n");
    }
}

fn write_rest(
    enums: &[EnumType],
    structs: &[Rc<StructType>],
    type_registry: &HashMap<String, Rc<StructType>>,
) {
    println!("use super::*;\n");
    println!("impl<'a> KeycloakAdmin<'a> {{");
    println!("}}");
}

fn text_opt(element: &ElementRef, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).unwrap();
    element.select(&selector).next().map(|x| element_text(&x))
}

fn text(element: &ElementRef, selector: &str) -> String {
    text_opt(element, selector).unwrap()
}

fn element_text(element: &ElementRef) -> String {
    element.text().collect::<Vec<_>>().join("")
}

struct StructType {
    name: String,
    is_camel_case: bool,
    fields: Vec<Field>,
}

struct EnumType {
    name: String,
    is_upper_case: bool,
    fields: Vec<String>,
}

impl StructType {
    fn is_lifetime(&self, type_registry: &HashMap<String, Rc<StructType>>) -> bool {
        self.fields
            .iter()
            .any(|x| x.field_type.is_lifetime(type_registry))
    }
}

struct Field {
    field_name: String,
    original_field: String,
    is_optional: bool,
    is_array: bool,
    is_rename: bool,
    field_type: FieldType,
}

enum FieldType {
    Simple(String),
    WithLifetime(String),
    Registry(String),
}

impl FieldType {
    fn is_lifetime(&self, type_registry: &HashMap<String, Rc<StructType>>) -> bool {
        match self {
            FieldType::WithLifetime(_) => true,
            FieldType::Registry(t) => type_registry
                .get(t)
                .map(|s| s.is_lifetime(type_registry))
                .unwrap_or_default(),
            FieldType::Simple(_) => false,
        }
    }

    fn name(&self, type_registry: &HashMap<String, Rc<StructType>>) -> String {
        match self {
            FieldType::Registry(name) => format!(
                "{}{}",
                name,
                if self.is_lifetime(type_registry) {
                    "<'a>"
                } else {
                    ""
                }
            ),
            FieldType::WithLifetime(name) | FieldType::Simple(name) => name.clone(),
        }
    }
}

struct Parameter {
    name: String,
    is_optional: bool,
    comment: Option<String>,
    kind: ParameterKind,
    is_array: bool,
    parameter_type: FieldType,
}

enum ParameterKind {
    Path,
    Query,
    Body,
    FormData,
}

impl FromStr for ParameterKind {
    type Err = String;
    fn from_str(value: &str) -> std::result::Result<Self, <Self as std::str::FromStr>::Err> {
        match value {
            "Path" => Ok(ParameterKind::Path),
            "Query" => Ok(ParameterKind::Query),
            "Body" => Ok(ParameterKind::Body),
            "FormData" => Ok(ParameterKind::FormData),
            _ => Err(format!("Unknown parameter kind: {}", value)),
        }
    }
}

struct MethodStruct {
    name: String,
    comment: String,
    path: String,
    parameters: Vec<Parameter>,
    method: String,
    response: ResponseType,
}

struct ResponseType {
    is_array: bool,
    return_type: FieldType,
}

fn convert_type(original: &str) -> Result<FieldType, ConvertTypeFail> {
    Ok(match original {
        "No Content" => FieldType::Simple("()".into()),
        "string" | "< string > array(csv)" => FieldType::WithLifetime("Cow<'a, str>".into()),
        "string(byte)" => FieldType::Simple("u8".into()),
        "integer(int32)" => FieldType::Simple("i32".into()),
        "integer(int64)" => FieldType::Simple("i64".into()),
        "number(float)" => FieldType::Simple("f32".into()),
        "boolean" => FieldType::Simple("bool".into()),
        "Map" => FieldType::WithLifetime("HashMap<Cow<'a, str>, Cow<'a, str>>".into()),
        "file" | "Object" => FieldType::Simple("Value".into()),
        _ => {
            if original.starts_with("enum (") {
                return Err(ConvertTypeFail::Enum(
                    (&original[6..original.len() - 1]).into(),
                ));
            } else if original.chars().next().unwrap().is_uppercase() {
                FieldType::Registry(original.into())
            } else {
                return Err(ConvertTypeFail::Unknown(original.into()));
            }
        }
    })
}

#[derive(Debug)]
enum ConvertTypeFail {
    Enum(String),
    Unknown(String),
}