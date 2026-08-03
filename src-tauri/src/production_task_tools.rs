use super::*;

pub(crate) fn register_production_task_tools() -> Result<(), OomuError> {
    connectors::register_task_tool().map_err(OomuError::Startup)?;
    artifacts::register_file_task_tool().map_err(OomuError::Startup)?;
    artifacts::workbooks::agent_tool::register_task_tool().map_err(OomuError::Startup)?;
    artifacts::presentations::register_presentation_task_tool().map_err(OomuError::Startup)?;
    decision_pack::register_task_tool().map_err(OomuError::Startup)?;
    tools::register_system_task_tools().map_err(OomuError::Startup)?;
    computer_use::register_task_tool().map_err(OomuError::Startup)?;
    gateway::register_task_tool().map_err(OomuError::Startup)?;
    Ok(())
}
