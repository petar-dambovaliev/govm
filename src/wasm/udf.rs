use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub functions: Vec<FunctionDescriptor>,
    pub aggregates: Vec<AggregateDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDescriptor {
    pub name: String,
    pub input: TableDescriptor,
    pub output: OutputDescriptor,
    pub returns_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregateDescriptor {
    pub name: String,
    pub input: TableDescriptor,
    pub output: OutputDescriptor,
    pub returns_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableDescriptor {
    pub fields: Vec<FieldDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDescriptor {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: String,
    pub nullable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OutputDescriptor {
    #[serde(rename = "scalar")]
    Scalar { scalar_type: String },
    #[serde(rename = "table")]
    Table { fields: Vec<FieldDescriptor> },
}

impl Manifest {
    pub fn new() -> Self {
        Self {
            functions: Vec::new(),
            aggregates: Vec::new(),
        }
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn validate_function(
        &self,
        name: &str,
        expected_inputs: &[(String, String)],
        expected_output: &str,
    ) -> Result<(), String> {
        let func = self
            .functions
            .iter()
            .find(|f| f.name == name)
            .ok_or_else(|| format!("function '{}' not found in manifest", name))?;

        if func.input.fields.len() != expected_inputs.len() {
            return Err(format!(
                "function '{}': expected {} input fields, got {}",
                name,
                expected_inputs.len(),
                func.input.fields.len()
            ));
        }

        for (i, (exp_name, exp_type)) in expected_inputs.iter().enumerate() {
            let field = &func.input.fields[i];
            if &field.name != exp_name {
                return Err(format!(
                    "function '{}': input field {}: expected name '{}', got '{}'",
                    name, i, exp_name, field.name
                ));
            }
            if &field.field_type != exp_type {
                return Err(format!(
                    "function '{}': input field '{}': expected type '{}', got '{}'",
                    name, exp_name, exp_type, field.field_type
                ));
            }
        }

        match &func.output {
            OutputDescriptor::Scalar { scalar_type } => {
                if scalar_type != expected_output {
                    return Err(format!(
                        "function '{}': expected return type '{}', got '{}'",
                        name, expected_output, scalar_type
                    ));
                }
            }
            OutputDescriptor::Table { fields } => {
                if expected_output != "table" {
                    return Err(format!(
                        "function '{}': expected scalar return '{}', got table return with {} fields",
                        name, expected_output, fields.len()
                    ));
                }
            }
        }

        Ok(())
    }

    pub fn validate_aggregate(
        &self,
        name: &str,
        expected_inputs: &[(String, String)],
        expected_output: &str,
    ) -> Result<(), String> {
        let agg = self
            .aggregates
            .iter()
            .find(|a| a.name == name)
            .ok_or_else(|| format!("aggregate '{}' not found in manifest", name))?;

        if agg.input.fields.len() != expected_inputs.len() {
            return Err(format!(
                "aggregate '{}': expected {} input fields, got {}",
                name,
                expected_inputs.len(),
                agg.input.fields.len()
            ));
        }

        for (i, (exp_name, exp_type)) in expected_inputs.iter().enumerate() {
            let field = &agg.input.fields[i];
            if &field.name != exp_name {
                return Err(format!(
                    "aggregate '{}': input field {}: expected name '{}', got '{}'",
                    name, i, exp_name, field.name
                ));
            }
            if &field.field_type != exp_type {
                return Err(format!(
                    "aggregate '{}': input field '{}': expected type '{}', got '{}'",
                    name, exp_name, exp_type, field.field_type
                ));
            }
        }

        match &agg.output {
            OutputDescriptor::Scalar { scalar_type } => {
                if scalar_type != expected_output {
                    return Err(format!(
                        "aggregate '{}': expected return type '{}', got '{}'",
                        name, expected_output, scalar_type
                    ));
                }
            }
            OutputDescriptor::Table { .. } => {
                return Err(format!(
                    "aggregate '{}': expected scalar return, got table return",
                    name
                ));
            }
        }

        Ok(())
    }
}
