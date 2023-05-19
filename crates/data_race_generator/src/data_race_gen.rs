use std::rc::Rc;

use serde_json::json;

use ast::types::{DataType, ScalarType, MemoryViewType};
use ast::{
    AccessMode, AssignmentLhs, AssignmentOp, AssignmentStatement, FnAttr, FnDecl, GlobalVarAttr,
    GlobalVarDecl, Module, ShaderStage, Statement, StorageClass, VarQualifier, ExprNode, Lit, 
    FnInput, FnInputAttr, VarDeclStatement, BinOp, VarExpr, BinOpExpr, Postfix, PostfixExpr, LetDeclStatement
};

use rand::distributions::{WeightedIndex};
use rand::prelude::{SliceRandom, StdRng};
use rand::Rng;
use rand_distr::Distribution;

use crate::Options;

#[derive(PartialEq, Eq, Copy, Clone)]
enum AccessType {
  ThreadSafe,
  ThreadUnsafe,
  ThreadRace,
  ConstantSafe,
  ConstantUnsafe,
  VarSafe,
  VarUnsafe,
  Literal
}

pub struct Outputs {
    pub safe: Module,
    pub race: Module,
    pub info: serde_json::Value
}

pub struct Generator<'a> {
    rng: &'a mut StdRng,
    options: Rc<Options>,
    safe_vars: Vec<String>,
    racy_vars: Vec<String>,
    lits: Vec<u32>,
    safe_offsets: Vec<u32>,
    racy_offsets: Vec<u32>,
    safe_constant_locs: Vec<u32>,
    racy_constant_locs: Vec<u32>,
    rhs_access_types: Vec<AccessType>,
    rhs_weights: WeightedIndex<i32>,
    safe_rhs_access_types: Vec<AccessType>,
    safe_rhs_weights: WeightedIndex<i32>,
    lhs_access_types: Vec<AccessType>,
    lhs_weights: WeightedIndex<i32>
}

impl<'a> Generator<'a> {
    pub fn new(rng: &'a mut StdRng, options: Rc<Options>) -> Self {

      let mut lhs_access_types = vec![];
      let mut rhs_access_types = vec![AccessType::Literal];
      let mut safe_rhs_access_types = vec![AccessType::Literal];

      let mut lits = vec![];
      // Create literals used in the program 
      for i in 0..options.num_lits {
        lits.push(i);
      }

      // The first N memory locations are divided into safe (read-only) and racy (read-write)
      let mut racy_constant_locs = vec![];
      let mut safe_constant_locs = vec![];
      for i in 0..options.constant_locs {
        if Self::rng_less_than(rng, options.racy_constant_loc_pct) {
          racy_constant_locs.push(i);
          Self::add_if_not_exists(&mut lhs_access_types, AccessType::ConstantUnsafe);
          Self::add_if_not_exists(&mut rhs_access_types, AccessType::ConstantUnsafe);
        } else {
          safe_constant_locs.push(i);
          Self::add_if_not_exists(&mut rhs_access_types, AccessType::ConstantSafe);
          Self::add_if_not_exists(&mut safe_rhs_access_types, AccessType::ConstantSafe);
        }
      }

      // Each thread's memory locations are divided into safe (read-write only by this thread)
      // and racy (read-write by any thread)
      let mut racy_offsets = vec![];
      let mut safe_offsets = vec![];

      for i in 0..options.locs_per_thread {
        if Self::rng_less_than(rng, options.racy_loc_pct) {
          racy_offsets.push(i);
          Self::add_if_not_exists(&mut lhs_access_types, AccessType::ThreadUnsafe);
          Self::add_if_not_exists(&mut lhs_access_types, AccessType::ThreadRace);
          Self::add_if_not_exists(&mut rhs_access_types, AccessType::ThreadUnsafe);
          Self::add_if_not_exists(&mut rhs_access_types, AccessType::ThreadRace);
        } else {
          safe_offsets.push(i);
          Self::add_if_not_exists(&mut lhs_access_types, AccessType::ThreadSafe);
          Self::add_if_not_exists(&mut rhs_access_types, AccessType::ThreadSafe);
          Self::add_if_not_exists(&mut safe_rhs_access_types, AccessType::ThreadSafe);
        }
      }

      // local variables are also divided into safe and racy
      let mut racy_vars = vec![];
      let mut safe_vars = vec![];
      for i in 0..options.vars {
        let name = format!("var_{i}");
        if Self::rng_less_than(rng, options.racy_var_pct) {
          racy_vars.push(name.to_owned());
          Self::add_if_not_exists(&mut lhs_access_types, AccessType::VarUnsafe);
          Self::add_if_not_exists(&mut rhs_access_types, AccessType::VarUnsafe);
        } else {
          safe_vars.push(name.to_owned());
          Self::add_if_not_exists(&mut lhs_access_types, AccessType::VarSafe);
          Self::add_if_not_exists(&mut rhs_access_types, AccessType::VarSafe);
          Self::add_if_not_exists(&mut safe_rhs_access_types, AccessType::VarSafe);
        }
      }

      let mut lhs_weight_values = vec![];
      for access_type in &lhs_access_types {
        lhs_weight_values.push(Self::lhs_access_type_weight(access_type));
      }
      let lhs_weights = WeightedIndex::new(lhs_weight_values).unwrap_or(WeightedIndex::new(vec![1]).unwrap());

      let mut rhs_weight_values = vec![];
      for access_type in &rhs_access_types {
        rhs_weight_values.push(Self::rhs_access_type_weight(access_type));
      }
      let rhs_weights = WeightedIndex::new(rhs_weight_values).unwrap_or(WeightedIndex::new(vec![1]).unwrap());

      let mut safe_rhs_weight_values = vec![];
      for access_type in &safe_rhs_access_types {
        safe_rhs_weight_values.push(Self::safe_rhs_access_type_weight(access_type));
      }
      let safe_rhs_weights = WeightedIndex::new(safe_rhs_weight_values).unwrap_or(WeightedIndex::new(vec![1]).unwrap());


      Generator {
        rng,
        options: options.clone(),
        safe_vars, 
        racy_vars,
        lits,
        safe_offsets,
        racy_offsets,
        safe_constant_locs,
        racy_constant_locs,
        rhs_access_types,
        rhs_weights,
        safe_rhs_access_types,
        safe_rhs_weights,
        lhs_access_types,
        lhs_weights
      }
    }

    fn add_if_not_exists<T: PartialEq>(vector: &mut Vec<T>, value: T) {
      if !vector.contains(&value) {
        vector.push(value);
      }
    }

    fn rng_less_than(rng: &mut StdRng, pct: u32) -> bool {
      return rng.gen_range(0..100) < pct;
    }

    fn lhs_access_type_weight(access_type: &AccessType) -> i32 {
      match access_type {
        AccessType::ThreadSafe => 17,
        AccessType::ThreadUnsafe => 17,
        AccessType::ThreadRace => 17,
        AccessType::ConstantUnsafe => 17,
        AccessType::VarSafe => 16,
        AccessType::VarUnsafe => 16,
        _ => 0
      }
    }

    fn rhs_access_type_weight(access_type: &AccessType) -> i32 {
      match access_type {
        AccessType::ThreadSafe => 13,
        AccessType::ThreadUnsafe => 13,
        AccessType::ThreadRace => 13,
        AccessType::ConstantSafe => 13,
        AccessType::ConstantUnsafe => 12,
        AccessType::VarSafe => 12,
        AccessType::VarUnsafe => 12,
        AccessType::Literal => 12
      }
    }

    fn safe_rhs_access_type_weight(access_type: &AccessType) -> i32 {
       match access_type {
        AccessType::ThreadSafe => 30,
        AccessType::ConstantSafe => 30,
        AccessType::VarSafe => 20,
        AccessType::Literal => 20,
        _ => 0
      }
    }

    #[tracing::instrument(skip(self))]
    pub fn gen_module(&mut self) -> Outputs {
        let global_vars = vec![
            GlobalVarDecl {
                attrs: vec![GlobalVarAttr::Group(0), GlobalVarAttr::Binding(0)],
                qualifier: Some(VarQualifier {
                    storage_class: StorageClass::Storage,
                    access_mode: Some(AccessMode::ReadWrite),
                }),
                name: "mem".to_owned(),
                data_type: DataType::array(ScalarType::U32, None),
                initializer: None,
            }
        ];


        let mut block: Vec<Statement> = vec![];

        // total ids is the number of workgroups times the workgroup size
        block.push(LetDeclStatement::new(
          "total_ids",
          BinOpExpr::new(
            BinOp::Times, 
            VarExpr::new("num_workgroups.x").into_node(DataType::from(ScalarType::U32)), 
            ExprNode::from(Lit::U32(self.options.workgroup_size)))).into());

        for var in self.safe_vars.to_owned() {
          block.push(self.initialize_var(var));
        }

        for var in self.racy_vars.to_owned() {
          block.push(self.initialize_var(var));
        }

        // Make a block to store statements that are safe
        let mut safe_block: Vec<Statement> = block.clone();

        for _i in 0..self.options.stmts {
          // Return a pair of AccessType + Statement
          // Then match, if safe add to both modules otherwise only to the race module
          let (stmt, access_type) = self.gen_statement();
          if self.safe_rhs_access_types.contains(&access_type) {
            safe_block.push(stmt.clone());
          }
          block.push(stmt);
        }

        let mut num_workgroups = FnInput::new("num_workgroups", DataType::Vector(3, ScalarType::U32));
        num_workgroups.attrs.push(FnInputAttr::Builtin("num_workgroups".to_string()));
        let mut local_invocation_id = FnInput::new("local_invocation_id", DataType::Vector(3, ScalarType::U32));
        local_invocation_id.attrs.push(FnInputAttr::Builtin("local_invocation_id".to_string()));
        let mut workgroup_id = FnInput::new("workgroup_id", DataType::Vector(3, ScalarType::U32));
        workgroup_id.attrs.push(FnInputAttr::Builtin("workgroup_id".to_string()));

        let entrypoint = FnDecl {
            attrs: vec![
                FnAttr::Stage(ShaderStage::Compute),
                FnAttr::LitWorkgroupSize(self.options.workgroup_size),
            ],
            name: "main".to_owned(),
            inputs: vec![num_workgroups.clone(), workgroup_id.clone(), local_invocation_id.clone()],
            output: None,
            body: block,
        };

        let safe_entrypoint = FnDecl {
            attrs: vec![
                FnAttr::Stage(ShaderStage::Compute),
                FnAttr::LitWorkgroupSize(self.options.workgroup_size),
            ],
            name: "main".to_owned(),
            inputs: vec![num_workgroups, workgroup_id, local_invocation_id],
            output: None,
            body: safe_block,
        };

        let functions = vec![entrypoint];
        let safe_functions = vec![safe_entrypoint];

        // Write the safe values to a json file to be read later
        let json_info = json!({
            "safe": self.safe_offsets
        });
        
        Outputs {
            safe: Module {
                    structs: vec![],
                    consts: vec![], 
                    vars: global_vars.clone(),
                    functions: safe_functions
                },
            race: Module {
                    structs: vec![],
                    consts: vec![],
                    vars: global_vars.clone(),
                    functions
                },
            info: json_info
        } 
    }

    fn initialize_var(&mut self, name: String) -> Statement {
      let ty = ScalarType::U32;
      VarDeclStatement::new(name, Some(ty.into()), None).into()
    }

    fn constant_expr(&mut self, choices: Vec<u32>) -> ExprNode {
      ExprNode::from(Lit::U32(choices.choose(self.rng).unwrap().to_owned()))
    }

    fn gen_mem_idx(&mut self, access_type: AccessType) -> ExprNode {
      let offset = match access_type {
        AccessType::ThreadSafe => self.safe_offsets.choose(self.rng).unwrap().to_owned(),
        AccessType::ThreadUnsafe | AccessType::ThreadRace => self.racy_offsets.choose(self.rng).unwrap().to_owned(),
        _ => 0 //unused
      };
      match access_type {
        AccessType::ConstantSafe => {
          let choices = self.safe_constant_locs.to_owned();
          self.constant_expr(choices)
        },
        AccessType::ConstantUnsafe => {
          let choices = self.racy_constant_locs.to_owned();
          self.constant_expr(choices)
        },
        AccessType::ThreadSafe | AccessType::ThreadUnsafe => {
          let base_id = BinOpExpr::new(
            BinOp::Times,
            VarExpr::new("local_invocation_id.x").into_node(DataType::from(ScalarType::U32)),
            ExprNode::from(Lit::U32(self.options.locs_per_thread)));
          BinOpExpr::new(
            BinOp::Plus,
            base_id,
            ExprNode::from(Lit::U32(offset + self.options.constant_locs))).into()
        },
        AccessType::ThreadRace => {
          let new_id = BinOpExpr::new(
            BinOp::Plus,
            VarExpr::new("local_invocation_id.x").into_node(DataType::from(ScalarType::U32)),
            ExprNode::from(Lit::U32(self.rng.gen_range(1..1024)))); //TODO: magic number

          let mod_id = BinOpExpr::new(
            BinOp::Mod,
            new_id,
            VarExpr::new("total_ids").into_node(DataType::from(ScalarType::U32)));

          let base_id = BinOpExpr::new(
            BinOp::Times,
            mod_id,
            ExprNode::from(Lit::U32(self.options.locs_per_thread)));
          BinOpExpr::new(
            BinOp::Plus,
            base_id,
            ExprNode::from(Lit::U32(offset))).into()
        }
        _ => unreachable!()
      }
    }

    fn gen_mem_access(&mut self, access_type: AccessType) -> ExprNode {
      let index = Postfix::index(self.gen_mem_idx(access_type));
      let arr_expr = VarExpr::new("mem").into_node(
        DataType::Ref(MemoryViewType::new(
    DataType::array(ScalarType::U32, None), StorageClass::Storage)));
      PostfixExpr::new(arr_expr, index).into()
    }

    fn var_expr(&mut self, choices: Vec<String>) -> ExprNode {
      VarExpr::new(choices.choose(self.rng).unwrap()).into_node(DataType::from(ScalarType::U32))
    }

    fn gen_op(&mut self, access_type: AccessType) -> ExprNode {
      match access_type {
        AccessType::Literal => ExprNode::from(Lit::U32(self.lits.choose(self.rng).unwrap().to_owned())),
        AccessType::VarSafe => {
          let choices = self.safe_vars.to_owned();
          self.var_expr(choices)
        },
        AccessType::VarUnsafe => {
          let choices = self.racy_vars.to_owned();
          self.var_expr(choices)
        },
        _ => self.gen_mem_access(access_type)
      }
    }

    fn gen_expr(&mut self, access_type: &AccessType) -> ExprNode {
      let left_access_type = if self.safe_rhs_access_types.contains(access_type) {
        self.safe_rhs_access_types[self.safe_rhs_weights.sample(self.rng)]
      } else {
        self.rhs_access_types[self.rhs_weights.sample(self.rng)]
      };

      let right_access_type = if self.safe_rhs_access_types.contains(access_type) {
        self.safe_rhs_access_types[self.safe_rhs_weights.sample(self.rng)]
      } else {
        self.rhs_access_types[self.rhs_weights.sample(self.rng)]
      };

      BinOpExpr::new(
        BinOp::Plus, 
        self.gen_op(left_access_type), 
        self.gen_op(right_access_type)).into()
    }

    fn gen_statement(&mut self) -> (Statement, AccessType) {
      let lhs_access_type: AccessType = self.lhs_access_types[self.lhs_weights.sample(self.rng)];
      let expr = self.gen_expr(&lhs_access_type.clone());
      let stmt = match lhs_access_type {
        AccessType::VarSafe => {
          let var = self.safe_vars.choose(self.rng).unwrap();
          AssignmentStatement::new(AssignmentLhs::name(var, DataType::from(ScalarType::U32)), AssignmentOp::Simple, expr).into()
        }
        AccessType::VarUnsafe => {
          let var = self.safe_vars.choose(self.rng).unwrap();
          AssignmentStatement::new(AssignmentLhs::name(var, DataType::from(ScalarType::U32)), AssignmentOp::Simple, expr).into()
        }
        _ => AssignmentStatement::new(
        AssignmentLhs::array_index(
          "mem", 
          DataType::Ref(MemoryViewType::new(
            DataType::array(ScalarType::U32, None), StorageClass::Storage)),
            self.gen_mem_idx(lhs_access_type)), 
        AssignmentOp::Simple,
        expr).into()
      };
      (stmt, lhs_access_type)
    }
}
