use arrow::array::{
    Array, BooleanArray, Float32Array, Float64Array, Int8Array, Int16Array, Int32Array, Int64Array,
    LargeStringArray, StringArray, UInt8Array, UInt16Array, UInt32Array, UInt64Array,
};
use arrow::datatypes::DataType;
use arrow::record_batch::RecordBatch;
use arrow::util::display::{ArrayFormatter, FormatOptions};
use serde_json::{Map, Value, json};

pub fn batches_to_json_rows(batches: &[RecordBatch]) -> Vec<Value> {
    let row_count = batches.iter().map(RecordBatch::num_rows).sum();
    let mut rows = Vec::with_capacity(row_count);

    for batch in batches {
        let schema = batch.schema();
        for row_index in 0..batch.num_rows() {
            let mut row = Map::with_capacity(batch.num_columns());
            for (column_index, field) in schema.fields().iter().enumerate() {
                row.insert(
                    field.name().clone(),
                    array_value(batch.column(column_index).as_ref(), row_index),
                );
            }
            rows.push(Value::Object(row));
        }
    }

    rows
}

fn array_value(array: &dyn Array, row: usize) -> Value {
    if array.is_null(row) {
        return Value::Null;
    }

    match array.data_type() {
        DataType::Boolean => value(array, row, BooleanArray::value),
        DataType::Int8 => value(array, row, Int8Array::value),
        DataType::Int16 => value(array, row, Int16Array::value),
        DataType::Int32 => value(array, row, Int32Array::value),
        DataType::Int64 => value(array, row, Int64Array::value),
        DataType::UInt8 => value(array, row, UInt8Array::value),
        DataType::UInt16 => value(array, row, UInt16Array::value),
        DataType::UInt32 => value(array, row, UInt32Array::value),
        DataType::UInt64 => value(array, row, UInt64Array::value),
        DataType::Float32 => value(array, row, Float32Array::value),
        DataType::Float64 => value(array, row, Float64Array::value),
        DataType::Utf8 => array
            .as_any()
            .downcast_ref::<StringArray>()
            .map(|typed| json!(typed.value(row)))
            .unwrap_or(Value::Null),
        DataType::LargeUtf8 => array
            .as_any()
            .downcast_ref::<LargeStringArray>()
            .map(|typed| json!(typed.value(row)))
            .unwrap_or(Value::Null),
        _ => ArrayFormatter::try_new(array, &FormatOptions::default())
            .map(|formatter| json!(formatter.value(row).to_string()))
            .unwrap_or(Value::Null),
    }
}

fn value<T, F, V>(array: &dyn Array, row: usize, getter: F) -> Value
where
    T: Array + 'static,
    F: Fn(&T, usize) -> V,
    V: serde::Serialize,
{
    array
        .as_any()
        .downcast_ref::<T>()
        .map(|typed| json!(getter(typed, row)))
        .unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{Int64Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};

    use super::*;

    #[test]
    fn renders_typed_json_rows() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int64Array::from(vec![1])),
                Arc::new(StringArray::from(vec!["signup"])),
            ],
        )
        .expect("record batch");

        assert_eq!(
            batches_to_json_rows(&[batch]),
            vec![json!({"id": 1, "name": "signup"})]
        );
    }

    #[test]
    fn renders_temporal_value_not_type_name() {
        use arrow::array::Date32Array;

        let schema = Arc::new(Schema::new(vec![Field::new("ts", DataType::Date32, false)]));
        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(Date32Array::from(vec![20605]))], // 2026-06-01
        )
        .expect("record batch");

        assert_eq!(
            batches_to_json_rows(&[batch]),
            vec![json!({"ts": "2026-06-01"})]
        );
    }
}
