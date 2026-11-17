//! Original `pctl` (parental controls) service modelling a console with no
//! parental controls configured: nothing is restricted and no pairing exists.

use emulator_ipc::Request;
use emulator_kernel::service::{Service, ServiceError, ServiceReply};

use crate::payload;

#[derive(Clone, Debug, Default)]
pub struct ParentalControlFactory;

impl Service for ParentalControlFactory {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        match request.command_id {
            // CreateService (0) / CreateServiceWithoutInitialize (1), both
            // taking the caller's process id.
            0 | 1 => {
                payload(request, 8)?;
                if request.pid.is_none() {
                    return Err(ServiceError::MissingSenderIdentity);
                }
                Ok(ServiceReply {
                    moved_objects: vec![Box::new(ParentalControlService)],
                    ..ServiceReply::default()
                })
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "pctl",
                command,
            }),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ParentalControlService;

impl Service for ParentalControlService {
    fn clone_for_request(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn dispatch(&mut self, request: &Request) -> Result<ServiceReply, ServiceError> {
        let flag = |value: bool| {
            Ok(ServiceReply {
                data: vec![u8::from(value)],
                ..ServiceReply::default()
            })
        };
        match request.command_id {
            // Initialize; permission checks that pass when nothing is restricted:
            // CheckFreeCommunicationPermission (1001),
            // ConfirmStereoVisionPermission (1013),
            // ConfirmStereoVisionRestrictionConfigurable (1061),
            // ResetConfirmedStereoVisionPermission (1064).
            1 | 1001 | 1013 | 1061 | 1064 => {
                payload(request, 0)?;
                Ok(ServiceReply::default())
            }
            // IsRestrictionEnabled, GetStereoVisionRestriction, IsPairingActive
            1031 | 1062 | 1403 => {
                payload(request, 0)?;
                flag(false)
            }
            // IsFreeCommunicationAvailable, IsStereoVisionPermitted
            1018 | 1065 => {
                payload(request, 0)?;
                flag(true)
            }
            command => Err(ServiceError::UnsupportedCommand {
                service: "pctl:IParentalControlService",
                command,
            }),
        }
    }
}
