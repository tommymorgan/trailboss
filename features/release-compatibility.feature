Feature: Manual release compatibility and version-impact evidence

  Scenario: RetainedReleaseClientWorksWithCandidateCore
    Given an identified retained Trailboss release contains the original client, consumers and producer artifacts with recorded digests
    And a compatible candidate Core runs in an isolated session with an original marker shell
    When the manual compatibility command runs the retained client and independent consumers against that Core
    Then the client renders terminal output, sends input to that original shell and detaches without stopping it
    And the retained API and plugin action consumers receive their supported results
    And the report identifies the compared revisions, product versions, artifact digests, protocol generations and codecs

  Scenario Outline: RegenerationCannotEraseReleasedContractDifferences
    Given the retained release baseline is unchanged
    And a candidate introduces <change> while its current encoder, decoder and generated artifacts agree
    When the manual compatibility command compares the candidate with the retained baseline
    Then it reports the affected released contract rather than declaring compatibility from current round trips
    And regenerating the candidate schema or fixtures cannot erase that difference
    Examples:
      | change |
      | a removed session API method |
      | a newly required request field |
      | a reordered binary payload field |
      | an appended enum variant emitted to an older reader |
      | an unknown widget node emitted to an older typed client |
      | changed action behavior with an unchanged request schema |

  Scenario Outline: MissingOrChangedReleasedArtifactsFailClosed
    Given the manual compatibility command requires a recorded retained Trailboss baseline
    And its required identity, version and artifact metadata must be valid
    When <problem> prevents establishing the original baseline
    Then the command exits unsuccessfully and identifies the missing, mismatched or invalid required evidence with artifact or metadata context
    And it does not substitute the candidate or an upstream Herdr release as the baseline
    Examples:
      | problem |
      | a required released binary is absent |
      | an artifact digest differs from its recorded digest |
      | source or product provenance is missing |
      | a required released schema or consumer is absent |
      | required metadata is malformed or inconsistent with the pinned contract evidence |

  Scenario Outline: ReportExplainsLikelyVersionImpact
    Given a baseline public contract version of 1.4.0 and a candidate with <change>
    When the manual compatibility command completes its comparison and supported probes
    Then its report suggests <bump> and identifies the affected contract and evidence
    And the suggestion applies only to the checked compatibility scope rather than choosing Tommy's release version
    Examples:
      | change | bump |
      | no detected public contract change | patch may suffice if the remaining changes are compatible fixes |
      | a backward-compatible public capability addition | minor |
      | removal of a supported API method | major |
      | an incompatible existing binary payload | major |
      | an incompatible existing widget contract | major |

  Scenario: NewCapabilitiesPreserveTheReleasedV1Contract
    Given a candidate adds a new negotiated capability or named codec while retaining the released generation-1 contract
    When the manual compatibility command exercises both the retained client and a capable candidate client
    Then the retained client continues using its original supported codecs and widget vocabulary
    And only the capable client is offered the new capability
    And the report explains the compatible addition without replacing or re-pinning the baseline

  Scenario: PublicBreakRemainsVisibleAfterAPackageBump
    Given a candidate changes an existing generation-1 payload incompatibly under its original codec name
    And its package or private protocol version has been increased
    When the manual compatibility command compares it with the retained baseline
    Then the report still identifies the incompatible public contract and its affected readers
    And it explains that a package bump does not restore the promised V1 path or negotiate a different codec

  Scenario Outline: ManualCheckDoesNotRequireReleaseAutomationOrRecordedDecision
    Given the baseline and candidate artifacts are available locally
    And no release pipeline, CI workflow, publisher or recorded version decision has been configured
    When Tommy invokes the documented compatibility test command for <comparison>
    Then it runs the comparison and probes and prints findings, version-impact guidance and uncertainty
    And it exits <exit_code> for <outcome>
    And guidance is available without choosing a version or recording a decision first
    And it enforces no release or version-selection policy and requires no recorded decision gate
    And it neither changes package versions nor publishes artifacts, creates tags or modifies release configuration
    Examples:
      | comparison | exit_code | outcome |
      | an unchanged supported public contract | 0 | checked compatibility scope |
      | an incompatible existing public contract | 1 | a detected contract difference |
      | an unclassified public schema difference | 1 | incomplete compatibility evidence |

  Scenario: CompatibilityReportDoesNotClaimUnexercisedContracts
    Given the compatibility command has exercised the retained plain-PTY consumer but no retained graphics, popup or optional delta consumer
    When the compatibility report is produced
    Then it identifies the exercised client/Core direction, methods, codecs and widget vocabulary
    And graphics, popup and optional delta compatibility are not marked verified from that plain-PTY result
    And missing evidence is reported as unverified rather than an unconditional compatible verdict or an invented version recommendation

  Scenario Outline: PreOneVersionPolicyIsNotInventedByTheCommand
    Given the compared product versions are below 1.0 and no pre-1.0 versioning policy has been selected
    And the candidate has <change>
    When the compatibility command reports the version impact
    Then it identifies <stable_implication> for a stable public contract within the checked scope
    And it states that the current 0.x version bump depends on Tommy's chosen policy rather than silently adopting one
    And it selects no pre-1.0 policy, product-version bump or release version
    And Tommy chooses and applies any pre-1.0 policy outside this command
    Examples:
      | change | stable_implication |
      | no detected public contract change | patch may suffice if the remaining changes are compatible fixes |
      | a backward-compatible public capability addition | minor |
      | a detected public compatibility break | major |
