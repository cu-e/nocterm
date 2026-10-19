use super::{field, missing, seeded_struct, unknown};
use crate::model::{auth::*, extensions::Extensions, ids::*, tools::ToolPatch};
seeded_struct!(PermissionChoice{id:PermissionOptionId => plain required,label:String => plain required,effect:PermissionEffect => plain required,extensions:Option<Extensions> => owned optional});
seeded_struct!(ProviderPermission{session:SessionId => plain required,tool:ToolPatch => owned required,choices:Vec<PermissionChoice> => owned required,extensions:Option<Extensions> => owned optional});
