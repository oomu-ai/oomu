use super::*;

#[test]
fn rejects_compiler_mapping_that_changes_the_declared_reference() {
    let output = json!({
        "compilerVersion": "1.0.0",
        "instructions": [{
            "nodeId": "agent",
            "systemPrompt": "Draft a response from the supplied request and return JSON.",
            "inputVariableMappings": [{
                "name": "request",
                "template": "{{nodes.untrusted.output}}"
            }],
            "evaluationProtocol": {
                "successCriteria": ["Output is valid JSON."],
                "failureAction": "fail",
                "maxRetries": 0
            }
        }]
    });

    let error = parse_compiler_output(&output.to_string(), &workflow_ir()).unwrap_err();
    assert_eq!(error.code, "workflow_compiler_contract_invalid");
    assert!(error.message.contains("must exactly match"));
}
