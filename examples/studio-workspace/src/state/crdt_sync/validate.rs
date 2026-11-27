//! Schema validation of decoded `NVC1` records before [`super::StudioCrdtDoc`] merges them: every
//! namespace, field, and value type must belong to the Studio schema, so a malformed packet is
//! rejected whole and the document stays untouched.

use nve_interop::crdt::{CrdtRecord, CrdtValue, CrdtWireError};

use super::{
    decode_priority, decode_status, FIELD_BLOCK_CHECKED, FIELD_BLOCK_KIND, FIELD_BLOCK_TEXT,
    FIELD_CHAT_AUTHOR, FIELD_CHAT_BODY, FIELD_CHAT_TIMESTAMP, FIELD_DOC_SLUG, FIELD_DOC_TITLE,
    FIELD_DOC_UPDATED_AT, FIELD_TASK_ASSIGNEE, FIELD_TASK_KEY, FIELD_TASK_PRIORITY, FIELD_TASK_STATUS,
    FIELD_TASK_TAG, FIELD_TASK_TITLE, NS_CHAT_DESIGN_SET, NS_CHAT_ENG_SET, NS_CHAT_FIELDS,
    NS_CHAT_REACTIONS, NS_DOC_BLOCKS_RGA, NS_DOC_BLOCK_FIELDS, NS_DOC_FIELDS, NS_DOC_SET,
    NS_TASK_FIELDS, NS_TASK_SET,
};

pub(super) fn validate_studio_records(records: &[CrdtRecord]) -> Result<(), CrdtWireError> {
    for rec in records {
        match rec {
            CrdtRecord::OrSetAddU64 { namespace, .. }
            | CrdtRecord::OrSetRemove { namespace, .. } => {
                if !matches!(
                    *namespace,
                    NS_TASK_SET | NS_CHAT_ENG_SET | NS_CHAT_DESIGN_SET | NS_DOC_SET
                ) {
                    return Err(CrdtWireError::InvalidPayload(
                        "unknown Studio OR-Set namespace",
                    ));
                }
            }
            CrdtRecord::LwwSet {
                namespace,
                field_id,
                value,
                ..
            } => match (*namespace, *field_id, value) {
                (
                    NS_TASK_FIELDS,
                    FIELD_TASK_KEY | FIELD_TASK_TITLE | FIELD_TASK_ASSIGNEE | FIELD_TASK_TAG,
                    CrdtValue::Utf8(_),
                )
                | (
                    NS_CHAT_FIELDS,
                    FIELD_CHAT_AUTHOR | FIELD_CHAT_TIMESTAMP | FIELD_CHAT_BODY,
                    CrdtValue::Utf8(_),
                )
                | (
                    NS_DOC_FIELDS,
                    FIELD_DOC_SLUG | FIELD_DOC_TITLE | FIELD_DOC_UPDATED_AT,
                    CrdtValue::Utf8(_),
                )
                | (NS_DOC_BLOCK_FIELDS, FIELD_BLOCK_TEXT, CrdtValue::Utf8(_))
                | (NS_DOC_BLOCK_FIELDS, FIELD_BLOCK_CHECKED, CrdtValue::Bool(_)) => {}
                (NS_TASK_FIELDS, FIELD_TASK_PRIORITY, CrdtValue::U32(v))
                    if decode_priority(*v).is_some() => {}
                (NS_TASK_FIELDS, FIELD_TASK_STATUS, CrdtValue::U32(v))
                    if decode_status(*v).is_some() => {}
                (NS_DOC_BLOCK_FIELDS, FIELD_BLOCK_KIND, CrdtValue::U32(v)) if *v <= 5 => {}
                _ => {
                    return Err(CrdtWireError::InvalidPayload(
                        "invalid Studio LWW field/value",
                    ));
                }
            },
            CrdtRecord::PnCounterSet { namespace, .. } => {
                if *namespace != NS_CHAT_REACTIONS {
                    return Err(CrdtWireError::InvalidPayload(
                        "unknown Studio PnCounter namespace",
                    ));
                }
            }
            CrdtRecord::RgaInsert {
                namespace, value, ..
            } => {
                if *namespace != NS_DOC_BLOCKS_RGA || !matches!(value, CrdtValue::U32(_)) {
                    return Err(CrdtWireError::InvalidPayload("invalid Studio RGA insert"));
                }
            }
            CrdtRecord::RgaDelete { namespace, .. } => {
                if *namespace != NS_DOC_BLOCKS_RGA {
                    return Err(CrdtWireError::InvalidPayload("invalid Studio RGA delete"));
                }
            }
            CrdtRecord::GCounterSet { .. } | CrdtRecord::OrSetAddString { .. } => {
                return Err(CrdtWireError::InvalidPayload(
                    "unsupported record kind for StudioCrdtDoc",
                ));
            }
        }
    }
    Ok(())
}
